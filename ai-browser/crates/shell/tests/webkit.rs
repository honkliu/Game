// This executable runs on the main thread: AppKit cannot run in Rust's test threads.
#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    use std::collections::HashSet;
    use std::io::{Read, Write};
    use std::time::{Duration, Instant};

    use ai_platform_contract::identity::{DocumentToken, SnapshotVersion};
    use browser::{ai::context, layout, pdf, platform, ui};
    use serde_json::Value;
    use tao::event::Event;
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tao::window::WindowBuilder;
    use wry::{PageLoadEvent, WebViewBuilder};

    #[derive(Debug)]
    enum Check {
        Ready(&'static str),
        Script(&'static str, String),
        Snapshot(Result<String, String>),
        Download(bool),
        PdfPoll,
        Timeout,
    }

    fn fail(message: impl std::fmt::Display) -> ! {
        eprintln!("WKWebView smoke test failed: {message}");
        std::process::exit(1);
    }

    platform::ensure_runtime().unwrap_or_else(|e| fail(e));
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("local document.pdf");
    let stream = "BT /F1 24 Tf 72 700 Td (WebKit PDF selectable text) Tj ET";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut document = "%PDF-1.4\n".to_string();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(document.len());
        document.push_str(&format!("{} 0 obj\n{object}\nendobj\n", i + 1));
    }
    let xref = document.len();
    document.push_str("xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        document.push_str(&format!("{offset:010} 00000 n \n"));
    }
    document.push_str(&format!(
        "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
    ));
    std::fs::write(&file, document).unwrap();
    let source = format!("file://{}", file.to_string_lossy().replace(' ', "%20"));
    let download_dir = dir.path().join("downloads");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let download_url = format!("http://{}/attachment", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.read(&mut [0u8; 8192]).unwrap();
        let body = b"downloaded by WebKit";
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"webkit.txt\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
        stream.write_all(body).unwrap();
    });

    let event_loop = EventLoopBuilder::<Check>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = WindowBuilder::new()
        .with_visible(false)
        .build(&event_loop)
        .unwrap();
    let mut views = std::collections::HashMap::new();
    for (i, (name, url)) in [
        ("shell", ui::SHELL_URL.to_string()),
        ("sidebar", "asset://shell/sidebar/sidebar.html".to_string()),
        ("page", String::new()),
        ("pdf", pdf::viewer_url(&source)),
        ("download", download_url),
    ]
    .into_iter()
    .enumerate()
    {
        let ready = proxy.clone();
        let completed = proxy.clone();
        let destination = download_dir.clone();
        let builder = WebViewBuilder::new()
            .with_bounds(layout::rect(
                (i as i32 % 2) * 400,
                (i as i32 / 2) * 300,
                400,
                300,
            ))
            .with_incognito(true)
            .with_custom_protocol("asset".into(), |_, request| ui::serve_embedded(&request))
            .with_asynchronous_custom_protocol("pdf".into(), |_, request, responder| {
                pdf::serve(request, responder)
            })
            .with_ipc_handler(|_| {})
            .with_download_started_handler(move |_, path| {
                *path = platform::macos::download_destination(path, &destination).unwrap();
                true
            })
            .with_download_completed_handler(move |_, _, ok| {
                let _ = completed.send_event(Check::Download(ok));
            })
            .with_on_page_load_handler(move |event, _| {
                if matches!(event, PageLoadEvent::Finished) {
                    let _ = ready.send_event(Check::Ready(name));
                }
            });
        let builder = platform::configure(builder, None, name == "shell" || name == "sidebar");
        let builder = if name == "page" {
            builder.with_html(r#"<html><head><base href="https://fixture.example/"></head><body>
                <p>A visible passage with enough text to become a cited block.</p>
                <p hidden>This hidden passage must never reach the assistant.</p>
                <div style="opacity:0"><p>This transparent passage must never reach the assistant.</p></div>
                <a href="/next">Follow this link</a>
                <input id="name"><button onclick="this.textContent='clicked'">Click</button>
                <script>window.originalStringify = JSON.stringify; JSON.stringify = () => 'page-forged';</script>
                </body></html>"#)
        } else {
            builder.with_url(&url)
        };
        let view = builder.build_as_child(&window).unwrap_or_else(|e| fail(e));
        platform::shape_webview(&view, &window, (0, 0, 400, 300), 6);
        views.insert(name, view);
    }
    let timeout = proxy.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(45));
        let _ = timeout.send_event(Check::Timeout);
    });
    let mut passed = HashSet::new();
    let mut temporary = Some(dir);
    event_loop.run(move |event, _, flow| {
        if passed.len() == 6 {
            *flow = ControlFlow::Exit;
            return;
        }
        *flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_secs(1));
        match event {
            Event::UserEvent(Check::Ready("download")) => {}
            Event::UserEvent(Check::Download(ok)) => {
                if !ok || std::fs::read(download_dir.join("webkit.txt")).unwrap_or_default() != b"downloaded by WebKit" {
                    fail("download did not reach its selected destination");
                }
                passed.insert("download");
            }
            Event::UserEvent(Check::Ready("page")) => {
                let done = proxy.clone();
                context::capture(&views["page"], move |result| { let _ = done.send_event(Check::Snapshot(result)); })
                    .unwrap_or_else(|e| fail(format!("{e:?}")));
            }
            Event::UserEvent(Check::Ready("pdf")) | Event::UserEvent(Check::PdfPoll) => {
                let done = proxy.clone();
                views["pdf"].evaluate_script_with_callback(
                    "document.querySelector('.textLayer')?.textContent || document.getElementById('status')?.textContent || ''",
                    move |result| { let _ = done.send_event(Check::Script("pdf", result)); },
                ).unwrap();
            }
            Event::UserEvent(Check::Ready(name)) => {
                let done = proxy.clone();
                views[name].evaluate_script_with_callback(
                    "typeof window.__native?.invoke === 'function' && typeof window.ipc?.postMessage === 'function'",
                    move |result| { let _ = done.send_event(Check::Script(name, result)); },
                ).unwrap();
            }
            Event::UserEvent(Check::Snapshot(result)) => {
                let payload: Value = serde_json::from_str(&result.unwrap_or_else(|e| fail(e))).unwrap();
                let context = context::parse(&payload, DocumentToken(1), SnapshotVersion(1), 1,
                    "https://fixture.example/", "Fixture").unwrap_or_else(|e| fail(format!("{e:?}")));
                let text = context.blocks.iter().map(|b| b.text.as_str()).collect::<Vec<_>>().join("\n");
                if !text.contains("visible passage") || text.contains("hidden passage") || text.contains("transparent passage") {
                    fail(format!("incorrect isolated-world extraction: {text}"));
                }
                if context.links.first().map(|l| l.url.as_str()) != Some("https://fixture.example/next") {
                    fail("missing extracted link");
                }
                passed.insert("context");
                let done = proxy.clone();
                views["page"].evaluate_script_with_callback(
                    "JSON.stringify = window.originalStringify; document.querySelector('button').click(); document.querySelector('input').value='Mac input'; document.querySelector('button').textContent === 'clicked' && document.querySelector('input').value === 'Mac input'",
                    move |result| { let _ = done.send_event(Check::Script("interaction", result)); },
                ).unwrap();
            }
            Event::UserEvent(Check::Script("pdf", result)) => {
                let text: String = serde_json::from_str(&result).unwrap_or_default();
                if text.contains("WebKit PDF selectable text") {
                    passed.insert("pdf");
                } else if text.contains("could not be opened") {
                    fail(text);
                } else {
                    let again = proxy.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(300));
                        let _ = again.send_event(Check::PdfPoll);
                    });
                }
            }
            Event::UserEvent(Check::Script(name, result)) => {
                if result != "true" { fail(format!("{name}: {result}")); }
                passed.insert(name);
            }
            Event::UserEvent(Check::Timeout) => fail(format!("timed out; passed {passed:?}")),
            _ => {}
        }
        if passed.len() == 6 {
            eprintln!("WKWebView smoke test passed: {passed:?}");
            views.clear();
            temporary.take().unwrap().close().unwrap();
            *flow = ControlFlow::Exit;
        }
    });
}
