//! PDF documents in page webviews.
//!
//! WebView2's built-in PDF viewer is a plugin: page scripts cannot see its
//! selection or its text, so the AI pane could neither ask about selected text nor
//! read the document. PDF links are therefore opened in our own viewer (pdf.js,
//! embedded under `ui/pdf/`) served from a `pdf` custom protocol, where the text
//! is ordinary DOM. The tab keeps showing the document's real URL.

use percent_encoding::{percent_decode_str, utf8_percent_encode, NON_ALPHANUMERIC};
use std::borrow::Cow;
use std::io::Read;
use wry::http::{header, Request, Response, StatusCode};
use wry::RequestAsyncResponder;

/// What a page webview sees as the viewer's origin (`pdf://` is mapped to this on
/// Windows when the HTTPS scheme option is off).
const VIEWER_BASE: &str = "http://pdf.localhost/";

/// A link to a PDF the viewer can open: http(s) or a local file whose path ends in
/// `.pdf`.
pub fn is_pdf_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("file:///")) {
        return false;
    }
    let path = lower.split(['?', '#']).next().unwrap_or("");
    path.ends_with(".pdf")
}

/// Whether a page URL is the bundled viewer itself.
pub fn is_viewer_url(url: &str) -> bool {
    url.starts_with(VIEWER_BASE)
}

/// The viewer page URL for a document.
pub fn viewer_url(src: &str) -> String {
    format!("{VIEWER_BASE}viewer.html?src={}", utf8_percent_encode(src, NON_ALPHANUMERIC))
}

/// The document behind a viewer URL (what the address bar should show).
pub fn source_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix(VIEWER_BASE)?.strip_prefix("viewer.html?")?;
    query_param(rest.split('#').next().unwrap_or(rest), "src")
}

fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| percent_decode_str(v).decode_utf8_lossy().into_owned())
    })
}

fn reply(status: StatusCode, content_type: &str, body: Cow<'static, [u8]>) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-store")
        .body(body)
        .unwrap_or_else(|_| Response::new(Cow::Borrowed(&[][..])))
}

fn failure(status: StatusCode, message: &str) -> Response<Cow<'static, [u8]>> {
    reply(status, "text/plain; charset=utf-8", Cow::Owned(message.as_bytes().to_vec()))
}

/// The `pdf` protocol: the embedded viewer files, and `/data?u=` which hands the
/// document's bytes to the viewer (fetched here, so the viewer is not subject to
/// the document site's CORS policy).
pub fn serve(request: Request<Vec<u8>>, responder: RequestAsyncResponder) {
    let path = request.uri().path().to_string();
    if path != "/data" {
        let name = path.trim_start_matches('/');
        let response = match crate::ui::embedded_file(&format!("pdf/{name}")) {
            Some((ct, bytes)) => reply(StatusCode::OK, ct, Cow::Borrowed(bytes)),
            None => failure(StatusCode::NOT_FOUND, "not found"),
        };
        responder.respond(response);
        return;
    }
    let source = request.uri().query().and_then(|q| query_param(q, "u")).unwrap_or_default();
    // A download can take long; never hold the UI thread.
    std::thread::spawn(move || {
        responder.respond(match fetch(&source) {
            Ok(bytes) => reply(StatusCode::OK, "application/pdf", Cow::Owned(bytes)),
            Err(e) => failure(StatusCode::BAD_GATEWAY, &e),
        });
    });
}

fn fetch(source: &str) -> Result<Vec<u8>, String> {
    if let Some(rest) = source.strip_prefix("file:///") {
        let path = percent_decode_str(rest).decode_utf8_lossy().into_owned();
        return std::fs::read(&path).map_err(|e| format!("cannot read {path}: {e}"));
    }
    if !(source.starts_with("http://") || source.starts_with("https://")) {
        return Err("unsupported address".to_string());
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(20))
        .timeout_read(std::time::Duration::from_secs(60))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130 Safari/537.36")
        .build();
    let response = agent.get(source).call().map_err(|e| match e {
        ureq::Error::Status(code, _) => format!("the server answered HTTP {code}"),
        ureq::Error::Transport(t) => format!("could not connect ({:?})", t.kind()),
    })?;
    let mut bytes = Vec::new();
    response.into_reader().read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_pdf_links() {
        assert!(is_pdf_url("https://x.org/a/paper.PDF"));
        assert!(is_pdf_url("https://x.org/a/paper.pdf?dl=1#page=3"));
        assert!(is_pdf_url("file:///C:/docs/a.pdf"));
        assert!(!is_pdf_url("https://x.org/pdf"));
        assert!(!is_pdf_url("https://x.org/a.pdf.html"));
        assert!(!is_pdf_url("http://pdf.localhost/viewer.html?src=x"));
    }

    #[test]
    fn viewer_url_round_trips_the_source() {
        let src = "https://x.org/a b/ü.pdf?x=1&y=2#page=4";
        assert_eq!(source_of(&viewer_url(src)).as_deref(), Some(src));
        assert_eq!(source_of("https://x.org/"), None);
    }
}
