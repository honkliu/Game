//! Persisted tool selection and explicitly configured HTTP tools.

use std::collections::BTreeMap;
use std::io::Read;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{
    mcp::Hub,
    store::Store,
    tools::{self, Kind, ToolDef},
    web,
};

const MAX_RESPONSE: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolConfig {
    #[serde(default)]
    pub enabled: BTreeMap<String, bool>,
    #[serde(default)]
    pub http: Vec<HttpTool>,
    #[serde(default)]
    pub overrides: BTreeMap<String, ToolOverride>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolOverride {
    pub name: String,
    pub title: String,
    pub description: String,
    pub parameters: Value,
}

impl ToolOverride {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() || self.name.len() > 64
            || !self.name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte)) {
            return Err("Use 1-64 letters, digits, _ or - for the model tool name.".into());
        }
        if self.title.trim().is_empty() || self.description.trim().is_empty() {
            return Err("A title and description are required.".into());
        }
        if self.parameters["type"] != "object" || !self.parameters["properties"].is_object() {
            return Err("Parameters must be a JSON Schema object with properties.".into());
        }
        if serde_json::to_string(self).map_err(|error| error.to_string())?.len() > 64 * 1024 {
            return Err("Tool definition exceeds 64 KiB.".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpTool {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub title: String,
    pub description: String,
    pub method: String,
    pub url: String,
    pub parameters: Value,
    #[serde(default)]
    pub body: Value,
    #[serde(default = "json_format")]
    pub response: String,
    #[serde(default)]
    pub read_only: bool,
}

fn json_format() -> String {
    "json".into()
}

pub fn load(store: &Store, hub: &Hub) -> Result<ToolConfig, String> {
    let mut config = store.load_config()?;
    if let Some(tools) = config.tools {
        return Ok(tools);
    }
    // One-time migration preserves the user's existing enabled tools, visibly on disk.
    hub.sync(&config.mcp_servers);
    let mut selection = ToolConfig::default();
    for tool in tools::builtins().into_iter().chain(hub.tools()) {
        selection.enabled.insert(tool.id, true);
    }
    selection.enabled.insert("web_search".into(), true);
    selection.http.push(HttpTool {
        id: "web_search".into(),
        name: "web_search".into(),
        title: "DuckDuckGo web search".into(),
        description: "Search the web. Returns titles, URLs and snippets.".into(),
        method: "GET".into(),
        url: "https://html.duckduckgo.com/html/?q={{query}}".into(),
        parameters: json!({ "type": "object", "properties": {
            "query": { "type": "string" },
            "limit": { "type": "integer", "minimum": 1, "maximum": 10, "default": 6 }
        }, "required": ["query"] }),
        body: Value::Null,
        response: "duckduckgo_html".into(),
        read_only: true,
    });
    config.tools = Some(selection.clone());
    store.save_config(&config)?;
    Ok(selection)
}

impl HttpTool {
    pub fn definition(&self) -> ToolDef {
        ToolDef {
            id: self.id.clone(),
            name: if self.name.is_empty() { self.id.clone() } else { self.name.clone() },
            description: self.description.clone(),
            parameters: self.parameters.clone(),
            kind: Kind::Http(self.clone()),
            write: !self.read_only,
            trusted: false,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !super::store::valid_id(&self.id)
            || self.id.starts_with("mcp_")
            || tools::builtins().iter().any(|tool| tool.name == self.id)
        {
            return Err(
                "Use a unique tool ID containing lowercase letters, digits, _ or -.".into(),
            );
        }
        ToolOverride { name: self.definition().name, title: self.title.clone(),
            description: self.description.clone(), parameters: self.parameters.clone() }.validate()?;
        if !matches!(self.method.as_str(), "GET" | "POST")
            || !matches!(self.response.as_str(), "json" | "text" | "duckduckgo_html")
        {
            return Err("Unsupported HTTP method or response format.".into());
        }
        if self.parameters["type"] != "object" || !self.parameters["properties"].is_object() {
            return Err("Parameters must be a JSON Schema object with properties.".into());
        }
        let sample = substitute(&self.url, |_| Ok("sample".into()), true)?;
        let parsed = url::Url::parse(&sample).map_err(|_| "Enter a valid HTTP(S) URL.")?;
        if !matches!(parsed.scheme(), "http" | "https")
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err("Use an HTTP(S) URL without embedded credentials.".into());
        }
        if serde_json::to_string(self)
            .map_err(|e| e.to_string())?
            .len()
            > 64 * 1024
        {
            return Err("Tool configuration exceeds 64 KiB.".into());
        }
        Ok(())
    }
}

pub fn definitions(config: &ToolConfig, hub: &Hub) -> Vec<ToolDef> {
    tools::builtins().into_iter().chain(hub.tools()).map(|mut tool| {
        if let Some(edit) = config.overrides.get(&tool.id) {
            tool.name = edit.name.clone();
            tool.description = edit.description.clone();
            tool.parameters = edit.parameters.clone();
        }
        tool
    }).chain(config.http.iter().map(HttpTool::definition)).collect()
}

pub fn validate_names(config: &ToolConfig, hub: &Hub) -> Result<(), String> {
    let mut names = BTreeMap::new();
    for tool in definitions(config, hub) {
        if let Some(previous) = names.insert(tool.name.clone(), tool.id.clone()) {
            return Err(format!("Tool name '{}' is already used by '{previous}'.", tool.name));
        }
    }
    for (id, edit) in &config.overrides {
        if !names.values().any(|known| known == id) && names.insert(edit.name.clone(), id.clone()).is_some() {
            return Err(format!("Tool name '{}' is already in use.", edit.name));
        }
    }
    Ok(())
}

pub fn catalog(store: &Store, hub: &Hub) -> (Vec<Value>, Option<String>) {
    let (config, error) = match load(store, hub) {
        Ok(config) => (config, None),
        Err(error) => (ToolConfig::default(), Some(error)),
    };
    let mut rows = Vec::new();
    let servers = store.config().mcp_servers;
    for tool in definitions(&config, hub) {
        let (source, target) = match &tool.kind {
            Kind::Browser => ("browser", format!("Built-in browser operation: {}", tool.id)),
            Kind::Mcp { server, tool } => {
                let endpoint = servers.iter().find(|entry| &entry.id == server).map(|entry| {
                    entry.url.clone().unwrap_or_else(|| format!("{} {}", entry.command.as_deref().unwrap_or(""), entry.args.join(" ")))
                }).unwrap_or_else(|| "not connected".into());
                ("mcp", format!("MCP server {server} at {endpoint}: {tool}"))
            }
            Kind::Http(http) => ("http", format!("{} {}", http.method, http.url)),
            _ => (
                "local",
                match tool.id.as_str() {
                    "fetch_url" => "GET the public URL supplied in arguments; no browser cookies",
                    "fetch_skill" => "Skills in this browser profile",
                    _ => "Stored results from this agent run",
                }
                .to_string(),
            ),
        };
        let title = match &tool.kind {
            Kind::Http(http) => http.title.clone(),
            _ => config.overrides.get(&tool.id).map(|edit| edit.title.clone()).unwrap_or_else(|| tool.name.clone()),
        };
        let mut row = json!({
            "id": tool.id, "name": tool.name, "title": title, "description": tool.description,
            "source": source, "target": target, "parameters": tool.parameters,
            "read_only": !tool.write,
            "enabled": config.enabled.get(&tool.id).copied().unwrap_or(false),
            "model_definition": tool.schema(),
        });
        if let Kind::Http(http) = &tool.kind { row["http"] = json!(http); }
        rows.push(row);
    }
    for (name, enabled) in &config.enabled {
        if name.starts_with("mcp_") && !rows.iter().any(|row| row["id"] == *name) {
            let edit = config.overrides.get(name);
            rows.push(json!({ "id": name, "name": edit.map(|edit| edit.name.as_str()).unwrap_or(name),
                "title": edit.map(|edit| edit.title.as_str()).unwrap_or(name), "enabled": enabled,
                "source": "mcp", "target": "MCP server not connected",
                "description": edit.map(|edit| edit.description.as_str()).unwrap_or("Previously discovered tool; reconnect its server in MCP settings."),
                "parameters": edit.map(|edit| edit.parameters.clone()).unwrap_or_else(|| json!({"type":"object","properties":{}})),
                "read_only": false }));
        }
    }
    (rows, error)
}

fn argument(config: &HttpTool, args: &Value, key: &str) -> Result<Value, String> {
    args.get(key)
        .or_else(|| {
            config.parameters["properties"]
                .get(key)
                .and_then(|p| p.get("default"))
        })
        .cloned()
        .ok_or_else(|| format!("Missing HTTP tool argument: {key}"))
}

fn substitute(
    text: &str,
    mut value: impl FnMut(&str) -> Result<String, String>,
    encode: bool,
) -> Result<String, String> {
    let mut result = String::new();
    let mut rest = text;
    while let Some((before, after)) = rest.split_once("{{") {
        let (key, tail) = after
            .split_once("}}")
            .ok_or("Unclosed {{argument}} placeholder.")?;
        result.push_str(before);
        let replacement = value(key)?;
        if encode {
            for byte in replacement.bytes() {
                if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
                    result.push(byte as char);
                } else {
                    result.push_str(&format!("%{byte:02X}"));
                }
            }
        } else {
            result.push_str(&replacement);
        }
        rest = tail;
    }
    result.push_str(rest);
    Ok(result)
}

fn render_body(config: &HttpTool, args: &Value, value: &Value) -> Result<Value, String> {
    match value {
        Value::String(text) => {
            if let Some(key) = text
                .strip_prefix("{{")
                .and_then(|s| s.strip_suffix("}}"))
                .filter(|key| !key.contains("{{"))
            {
                argument(config, args, key)
            } else {
                substitute(
                    text,
                    |key| {
                        argument(config, args, key).map(|v| {
                            v.as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| v.to_string())
                        })
                    },
                    false,
                )
                .map(Value::String)
            }
        }
        Value::Array(values) => values
            .iter()
            .map(|value| render_body(config, args, value))
            .collect(),
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| Ok((key.clone(), render_body(config, args, value)?)))
            .collect::<Result<serde_json::Map<_, _>, String>>()
            .map(Value::Object),
        _ => Ok(value.clone()),
    }
}

pub fn execute(config: &HttpTool, args: &Value) -> Result<Value, String> {
    let url = substitute(
        &config.url,
        |key| {
            argument(config, args, key).map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| v.to_string())
            })
        },
        true,
    )?;
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .timeout(Duration::from_secs(45))
        .build();
    let request = agent
        .request(&config.method, &url)
        .set("Accept", "application/json,text/html,text/plain");
    let response = if config.method == "POST" {
        request
            .set("Content-Type", "application/json")
            .send_string(&render_body(config, args, &config.body)?.to_string())
    } else {
        request.call()
    }
    .map_err(|error| error.to_string())?;
    if (300..400).contains(&response.status()) {
        return Err("HTTP tool redirected; configure the final endpoint URL.".into());
    }
    let content_type = response.content_type().to_string();
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_RESPONSE + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_RESPONSE {
        return Err("HTTP tool response exceeds 2 MiB.".into());
    }
    let text = String::from_utf8_lossy(&bytes);
    match config.response.as_str() {
        "json" => {
            serde_json::from_str(&text).map_err(|error| format!("Invalid JSON response: {error}"))
        }
        "duckduckgo_html" => {
            let limit = argument(config, args, "limit").unwrap_or(json!(6))
                .as_u64()
                .unwrap_or(6)
                .clamp(1, 10) as usize;
            let rows = web::search_results(&text, limit);
            Ok(Value::String(if rows.is_empty() {
                "No results.".into()
            } else {
                rows.iter()
                    .enumerate()
                    .map(|(i, (title, url, snippet))| {
                        format!("{}. {title}\n   {url}\n   {snippet}", i + 1)
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }))
        }
        _ => Ok(Value::String(if content_type.contains("html") {
            web::html_to_text(&text).1
        } else {
            text.into_owned()
        })),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    fn profile(tag: &str) -> (std::path::PathBuf, Store) {
        let dir = std::env::temp_dir().join(format!(
            "agent-tools-{tag}-{}-{}",
            std::process::id(),
            crate::control::now_ms()
        ));
        fs::create_dir_all(&dir).unwrap();
        let store = Store::new(&dir);
        (dir, store)
    }

    fn search() -> HttpTool {
        HttpTool {
            id: "web_search".into(),
            name: "web_search".into(),
            title: "MoonShot".into(),
            description: "Search documents.".into(),
            method: "POST".into(),
            url: "http://localhost:3010/v1/search".into(),
            parameters: json!({"type":"object","properties":{
                    "query":{"type":"string"}, "limit":{"type":"integer","default":10}
                },"required":["query"]}),
            body: json!({"mode":"text","query":"{{query}}","limit":"{{limit}}","offset":0}),
            response: "json".into(),
            read_only: true,
        }
    }

    #[test]
    fn migration_is_visible_and_disabled_choices_survive_reload() {
        let (dir, store) = profile("migration");
        let hub = Hub::default();
        let migrated = load(&store, &hub).unwrap();
        assert!(migrated.enabled.values().all(|enabled| *enabled));
        assert_eq!(migrated.http[0].id, "web_search");
        assert!(migrated.http[0].url.contains("duckduckgo"));
        let state = super::super::admin::state(&store, &hub);
        assert!(state["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["id"] == "fetch_url"));
        let models = crate::provider::Catalog::from_config(
            &ai_platform_contract::config::AiConfig::default(),
        );
        super::super::admin::handle(
            &store,
            &hub,
            &models,
            &json!({"op":"tools.enable","id":"web_search","enabled":false}),
        )
        .unwrap();
        let reloaded = Store::new(&dir);
        assert!(!load(&reloaded, &hub).unwrap().enabled["web_search"]);
        assert_eq!(load(&reloaded, &hub).unwrap().http.len(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn explicit_empty_selection_and_corrupt_config_never_restore_defaults() {
        let (dir, store) = profile("empty");
        let hub = Hub::default();
        let mut config = store.config();
        config.tools = Some(ToolConfig::default());
        store.save_config(&config).unwrap();
        assert_eq!(load(&store, &hub).unwrap(), ToolConfig::default());
        assert!(catalog(&store, &hub)
            .0
            .iter()
            .all(|tool| tool["enabled"] == false));
        fs::write(dir.join("agent.json"), "{invalid").unwrap();
        assert!(load(&store, &hub).is_err());
        assert!(catalog(&store, &hub).1.is_some());
        assert_eq!(
            fs::read_to_string(dir.join("agent.json")).unwrap(),
            "{invalid"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn templates_preserve_json_types_escaping_defaults_and_url_arguments() {
        let mut config = search();
        let query = "树先生 \"quotes\"\n?& #/";
        let args = json!({"query":query});
        let body = render_body(&config, &args, &config.body).unwrap();
        assert_eq!(body["query"], query);
        assert_eq!(body["limit"], 10);
        assert_eq!(
            render_body(&config, &args, &json!(["{{limit}}", "prefix {{query}}"])).unwrap(),
            json!([10, format!("prefix {query}")])
        );
        assert!(render_body(&config, &json!({}), &config.body).is_err());
        config.url = "https://example.test/search?q={{query}}".into();
        let rendered = substitute(&config.url, |_| Ok(query.into()), true).unwrap();
        let parsed = url::Url::parse(&rendered).unwrap();
        assert_eq!(
            parsed.query_pairs().collect::<Vec<_>>(),
            vec![("q".into(), query.into())]
        );
    }

    #[test]
    fn saves_validate_ids_methods_schemas_and_urls() {
        let valid = search();
        valid.validate().unwrap();
        for id in ["fetch_url", "mcp_server__search", "../bad"] {
            let mut config = valid.clone();
            config.id = id.into();
            assert!(config.validate().is_err(), "{id}");
        }
        let mut config = valid.clone();
        config.url = "file:///local".into();
        assert!(config.validate().is_err());
        config = valid.clone();
        config.method = "DELETE".into();
        assert!(config.validate().is_err());
        config = valid;
        config.parameters = json!([]);
        assert!(config.validate().is_err());
    }

    #[test]
    fn definition_edits_preserve_handler_ids_and_flags_and_reject_duplicate_names() {
        let (dir, store) = profile("definitions");
        let hub = Hub::default();
        let models = crate::provider::Catalog::from_config(&ai_platform_contract::config::AiConfig::default());
        load(&store, &hub).unwrap();
        super::super::admin::handle(&store, &hub, &models,
            &json!({"op":"tools.enable","id":"read_page","enabled":false})).unwrap();
        let edit = json!({
            "name":"ReadCurrentPage", "title":"Read the current page",
            "description":"Edited page reader description.",
            "parameters":{"type":"object","properties":{"tab_id":{"type":"integer","description":"Edited target tab."}}}
        });
        let result = super::super::admin::handle(&store, &hub, &models,
            &json!({"op":"tools.definition.save","id":"read_page","definition":edit})).unwrap();
        let row = result["tools"].as_array().unwrap().iter().find(|row| row["id"] == "read_page").unwrap();
        assert_eq!(row["enabled"], false);
        assert_eq!(row["model_definition"]["function"]["name"], "ReadCurrentPage");
        let selected = load(&store, &hub).unwrap();
        let definition = definitions(&selected, &hub).into_iter().find(|tool| tool.id == "read_page").unwrap();
        assert_eq!(definition.name, "ReadCurrentPage");
        assert_eq!(definition.kind, Kind::Browser);
        assert_eq!(definition.parameters["properties"]["tab_id"]["description"], "Edited target tab.");
        assert!(super::super::admin::handle(&store, &hub, &models,
            &json!({"op":"tools.definition.save","id":"fetch_url","definition":edit})).is_err());
        let mut http = search();
        http.name = "ReadCurrentPage".into();
        assert!(super::super::admin::handle(&store, &hub, &models,
            &json!({"op":"tools.http.save","tool":http,"editing":true,"enabled":true})).is_err());
        let stored = load(&store, &hub).unwrap();
        assert!(!stored.overrides.contains_key("fetch_url"));
        assert!(stored.http[0].url.contains("duckduckgo"));
        super::super::admin::handle(&store, &hub, &models,
            &json!({"op":"tools.definition.reset","id":"read_page"})).unwrap();
        let restored = load(&store, &hub).unwrap();
        assert!(!restored.enabled["read_page"]);
        assert!(!restored.overrides.contains_key("read_page"));
        assert_eq!(definitions(&restored, &hub).into_iter().find(|tool| tool.id == "read_page").unwrap().name, "read_page");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn configured_http_search_and_document_lookup_use_the_configured_requests() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for response in [
                r#"{"documents":[{"document_id":"doc /1"}]}"#,
                r#"{"content":"Complete document"}"#,
                r#"<a class="result__a" href="https://example.test/">Result title</a><a class="result__snippet">Result snippet</a>"#,
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            length = value.trim().parse::<usize>().unwrap();
                        }
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                requests.push((request, body));
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
            }
            requests
        });
        let mut config = search();
        config.url = format!("{base}/v1/search");
        assert_eq!(
            execute(&config, &json!({"query":"树先生"})).unwrap()["documents"][0]["document_id"],
            "doc /1"
        );
        config.method = "GET".into();
        config.url = format!("{base}/v1/documents/{{{{document_id}}}}");
        assert_eq!(
            execute(&config, &json!({"document_id":"doc /1"})).unwrap()["content"],
            "Complete document"
        );
        config.url = format!("{base}/search?q={{{{query}}}}");
        config.response = "duckduckgo_html".into();
        config.parameters = json!({"type":"object","properties":{"query":{"type":"string"}}});
        let parsed = execute(&config, &json!({"query":"fixture"})).unwrap();
        assert!(parsed.as_str().unwrap().contains("Result title\n   https://example.test/\n   Result snippet"));
        let requests = server.join().unwrap();
        assert_eq!(requests[0].0, "POST /v1/search HTTP/1.1\r\n");
        assert_eq!(
            serde_json::from_slice::<Value>(&requests[0].1).unwrap(),
            json!({"mode":"text","query":"树先生","limit":10,"offset":0})
        );
        assert_eq!(requests[1].0, "GET /v1/documents/doc%20%2F1 HTTP/1.1\r\n");
        assert!(requests[1].1.is_empty());
    }
}
