//! PDF documents in page webviews.
//!
//! Windows uses WebView2's native viewer and its Windows accessibility provider
//! (`native_pdf`) for River's text and selection access. The bundled PDF.js viewer
//! remains the macOS adapter until a native PDFKit bridge is implemented.

use percent_encoding::{percent_decode_str, utf8_percent_encode, NON_ALPHANUMERIC};
use std::borrow::Cow;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use wry::http::{header, Request, Response, StatusCode};
use wry::RequestAsyncResponder;

const MAX_PDF_BYTES: usize = 64 * 1024 * 1024;
static ACTIVE_FETCHES: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Default)]
pub struct PdfSession(Arc<Mutex<Option<Arc<AtomicBool>>>>);

impl PdfSession {
    pub fn cancel(&self) {
        if let Some(cancelled) = self.0.lock().unwrap_or_else(|e| e.into_inner()).take() {
            cancelled.store(true, Ordering::Relaxed);
        }
    }

    fn begin(&self) -> Arc<AtomicBool> {
        let cancelled = Arc::new(AtomicBool::new(false));
        if let Some(previous) = self.0.lock().unwrap_or_else(|e| e.into_inner()).replace(cancelled.clone()) {
            previous.store(true, Ordering::Relaxed);
        }
        cancelled
    }
}

struct FetchPermit;
impl Drop for FetchPermit {
    fn drop(&mut self) { ACTIVE_FETCHES.fetch_sub(1, Ordering::Relaxed); }
}

/// What a page webview sees as the viewer's origin (`pdf://` is mapped to this on
/// Windows with the HTTPS scheme option enabled).
#[cfg(windows)]
const VIEWER_BASE: &str = "https://pdf.localhost/";
#[cfg(target_os = "macos")]
const VIEWER_BASE: &str = "pdf://localhost/";

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

#[cfg(any(windows, test))]
pub fn dropped_file_url(path: &std::path::Path) -> Option<String> {
    if !path.extension()?.to_str()?.eq_ignore_ascii_case("pdf") || !path.is_file() {
        return None;
    }
    let url = url::Url::from_file_path(path).ok()?.to_string();
    is_pdf_url(&url).then_some(url)
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
    serve_authenticated(request, responder, Vec::new(), &PdfSession::default());
}

pub fn requested_source(request: &Request<Vec<u8>>) -> String {
    request.uri().query().and_then(|q| query_param(q, "u")).unwrap_or_default()
}

pub fn serve_authenticated(request: Request<Vec<u8>>, responder: RequestAsyncResponder,
    cookies: Vec<wry::cookie::Cookie<'static>>, session: &PdfSession) {
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
    let source = requested_source(&request);
    if ACTIVE_FETCHES.compare_exchange(0, 1, Ordering::Relaxed, Ordering::Relaxed).is_err()
        && ACTIVE_FETCHES.compare_exchange(1, 2, Ordering::Relaxed, Ordering::Relaxed).is_err() {
        responder.respond(failure(StatusCode::TOO_MANY_REQUESTS, "Two PDFs are already loading; retry shortly."));
        return;
    }
    let permit = FetchPermit;
    let cancelled = session.begin();
    // A download can take long; never hold the UI thread.
    std::thread::spawn(move || {
        let _permit = permit;
        responder.respond(match fetch_authenticated(&source, &cookies, &cancelled) {
            Ok(bytes) => reply(StatusCode::OK, "application/pdf", Cow::Owned(bytes)),
            Err(e) => failure(StatusCode::BAD_GATEWAY, &e),
        });
    });
}

#[cfg(test)]
fn fetch(source: &str) -> Result<Vec<u8>, String> {
    fetch_authenticated(source, &[], &AtomicBool::new(false))
}

fn read_bounded(mut reader: impl Read, cancelled: &AtomicBool, started: Instant) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 32 * 1024];
    loop {
        if cancelled.load(Ordering::Relaxed) { return Err("PDF loading cancelled".into()); }
        if started.elapsed() > Duration::from_secs(90) { return Err("PDF loading timed out".into()); }
        let count = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 { return Ok(bytes); }
        if bytes.len() + count > MAX_PDF_BYTES { return Err("PDF exceeds the 64 MiB limit".into()); }
        bytes.extend_from_slice(&buffer[..count]);
    }
}

fn cookie_header(cookies: &[wry::cookie::Cookie<'static>], path: &str) -> String {
    let mut matching: Vec<_> = cookies.iter().filter(|cookie| {
        let scope = cookie.path().unwrap_or("/");
        path == scope || (path.starts_with(scope) && (scope.ends_with('/') || path.as_bytes().get(scope.len()) == Some(&b'/')))
    }).collect();
    matching.sort_by_key(|cookie| std::cmp::Reverse(cookie.path().unwrap_or("/").len()));
    matching.into_iter().map(|cookie| format!("{}={}", cookie.name(), cookie.value())).collect::<Vec<_>>().join("; ")
}

fn fetch_authenticated(source: &str, cookies: &[wry::cookie::Cookie<'static>], cancelled: &AtomicBool) -> Result<Vec<u8>, String> {
    let started = Instant::now();
    let initial = url::Url::parse(source).map_err(|_| "invalid PDF address")?;
    if initial.scheme() == "file" {
        let path = initial.to_file_path().map_err(|_| "invalid local PDF address")?;
        let file = std::fs::File::open(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        return read_bounded(file, cancelled, started);
    }
    if !matches!(initial.scheme(), "http" | "https") || !initial.username().is_empty() || initial.password().is_some() {
        return Err("unsupported PDF address".into());
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(3))
        .timeout(Duration::from_secs(90))
        .redirects(0)
        .user_agent("AI-Browser/0.1")
        .build();
    let mut current = initial.clone();
    for _ in 0..=5 {
        if cancelled.load(Ordering::Relaxed) { return Err("PDF loading cancelled".into()); }
        let mut request = agent.get(current.as_str()).set("Accept", "application/pdf");
        if current.origin() == initial.origin() {
            let header = cookie_header(cookies, current.path());
            if !header.is_empty() { request = request.set("Cookie", &header); }
        }
        let response = request.call().map_err(|e| match e {
            ureq::Error::Status(code, _) => format!("the server answered HTTP {code}"),
            ureq::Error::Transport(t) => format!("could not connect ({:?})", t.kind()),
        })?;
        if matches!(response.status(), 301 | 302 | 303 | 307 | 308) {
            let next = current.join(response.header("Location").ok_or("redirect has no destination")?)
                .map_err(|_| "invalid redirect")?;
            if !matches!(next.scheme(), "http" | "https") || (current.scheme() == "https" && next.scheme() != "https")
                || !next.username().is_empty() || next.password().is_some() {
                return Err("unsafe PDF redirect".into());
            }
            current = next;
            continue;
        }
        if response.header("Content-Length").and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|size| size > MAX_PDF_BYTES as u64) {
            return Err("PDF exceeds the 64 MiB limit".into());
        }
        let bytes = read_bounded(response.into_reader(), cancelled, started)?;
        if !bytes.windows(5).take(1024).any(|window| window == b"%PDF-") {
            return Err("the server returned a non-PDF response (sign in to the site and retry)".into());
        }
        return Ok(bytes);
    }
    Err("too many PDF redirects".into())
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
    fn dropped_pdfs_keep_their_real_escaped_file_address() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("document # 100%.PDF");
        std::fs::write(&path, b"%PDF-1.4").unwrap();
        let url = dropped_file_url(&path).unwrap();
        assert!(is_pdf_url(&url));
        assert_eq!(url::Url::parse(&url).unwrap().to_file_path().unwrap(), path);
        assert!(url.contains("%23"));
        assert!(url.contains("%25"));
    }

    #[test]
    fn drops_ignore_missing_files_directories_and_other_formats() {
        let dir = tempfile::tempdir().unwrap();
        let text = dir.path().join("notes.txt");
        std::fs::write(&text, b"notes").unwrap();
        let folder = dir.path().join("folder.pdf");
        std::fs::create_dir(&folder).unwrap();
        for path in [text, folder, dir.path().join("missing.pdf")] {
            assert!(dropped_file_url(&path).is_none());
        }
    }

    #[test]
    fn viewer_url_round_trips_the_source() {
        let src = "https://x.org/a b/ü.pdf?x=1&y=2#page=4";
        assert_eq!(source_of(&viewer_url(src)).as_deref(), Some(src));
        assert_eq!(source_of("https://x.org/"), None);
    }

    #[test]
    fn reads_local_pdf_files_with_escaped_names() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("local document.pdf");
        std::fs::write(&file, b"%PDF-1.4 local").unwrap();
        let path = file.to_string_lossy().replace('\\', "/");
        let prefix = if cfg!(windows) { "file:///" } else { "file://" };
        let url = format!("{prefix}{}", path.replace(' ', "%20"));
        assert_eq!(fetch(&url).unwrap(), b"%PDF-1.4 local");
        assert!(is_pdf_url(&url));
    }
}
