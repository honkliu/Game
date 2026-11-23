use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAlert, NSApplication, NSEvent, NSEventMask, NSEventModifierFlags, NSEventType, NSMenu,
    NSMenuItem, NSScreen, NSWindow,
};
use objc2_foundation::{NSError, NSProcessInfo, NSString};
use objc2_web_kit::WKContentWorld;
use tao::event_loop::EventLoopProxy;
use tao::platform::macos::WindowExtMacOS;
use tao::window::Window;
use wry::{WebView, WebViewExtMacOS};

use crate::app::AppMessage;
use crate::keys::{self, Outcome};

pub type Environment = [u8; 16];
static ENVIRONMENTS: AtomicU64 = AtomicU64::new(0);

pub fn environments_constructed() -> u64 {
    ENVIRONMENTS.load(Ordering::Relaxed)
}

pub fn ensure_runtime() -> Result<String, String> {
    let version = NSProcessInfo::processInfo().operatingSystemVersion();
    if version.majorVersion < 14 {
        let error = "AI Browser requires macOS 14 (Sonoma) or newer.".to_string();
        show_fatal(&error);
        return Err(error);
    }
    wry::webview_version().map_err(|e| e.to_string())
}

pub fn show_fatal(message: &str) {
    eprintln!("AI Browser: {message}");
    if let Some(mtm) = MainThreadMarker::new() {
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str("AI Browser"));
        alert.setInformativeText(&NSString::from_str(message));
        alert.runModal();
    }
}

pub fn create_shared_environment(data_dir: &Path) -> Result<Environment, String> {
    std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    let path = data_dir.join("profile-id");
    let id = match std::fs::read(&path) {
        Ok(bytes) => bytes
            .try_into()
            .map_err(|_| "invalid WebKit profile identifier".to_string())?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut id = [0; 16];
            std::fs::File::open("/dev/urandom")
                .and_then(|mut f| f.read_exact(&mut id))
                .map_err(|e| e.to_string())?;
            id[6] = (id[6] & 0x0f) | 0x40;
            id[8] = (id[8] & 0x3f) | 0x80;
            let name = id.iter().map(|b| format!("{b:02x}")).collect::<String>();
            let temporary = data_dir.join(format!(".profile-{name}.tmp"));
            let publish = (|| {
                let mut file = std::fs::File::create_new(&temporary)?;
                file.write_all(&id)?;
                file.sync_all()?;
                // Publish only complete identifiers, without replacing another launch's ID.
                match std::fs::hard_link(&temporary, &path) {
                    Ok(()) => Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
                    Err(e) => Err(e),
                }
            })();
            let _ = std::fs::remove_file(&temporary);
            publish.map_err(|e: std::io::Error| e.to_string())?;
            std::fs::read(&path)
                .map_err(|e| e.to_string())?
                .try_into()
                .map_err(|_| "invalid WebKit profile identifier".to_string())?
        }
        Err(e) => return Err(e.to_string()),
    };
    ENVIRONMENTS.fetch_add(1, Ordering::Relaxed);
    Ok(id)
}

pub fn download_destination(suggested: &Path, directory: &Path) -> std::io::Result<PathBuf> {
    let name = crate::downloads::sanitize_file_name(
        suggested
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("download"),
    );
    std::fs::create_dir_all(directory)?;
    Ok(directory.join(crate::downloads::unique_name(directory, &name)))
}

pub fn shape_webview(webview: &WebView, radius: f64) {
    let view = webview.webview();
    view.setWantsLayer(true);
    // SAFETY: the live WKWebView owns its backing CALayer; all calls are on the UI thread.
    unsafe {
        let layer: *mut AnyObject = msg_send![&*view, layer];
        if !layer.is_null() {
            let _: () = msg_send![layer, setCornerRadius: radius];
            let _: () = msg_send![layer, setMasksToBounds: true];
        }
    }
}

pub fn capture(
    webview: &WebView,
    script: &str,
    on_done: impl FnOnce(Result<String, String>) + Send + 'static,
) {
    let Some(mtm) = MainThreadMarker::new() else {
        on_done(Err("page capture requires the main thread".into()));
        return;
    };
    let on_done = std::cell::RefCell::new(Some(on_done));
    let completion = RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
        let Some(on_done) = on_done.borrow_mut().take() else {
            return;
        };
        // SAFETY: WebKit supplies valid callback objects for this invocation.
        let result = unsafe {
            if let Some(error) = error.as_ref() {
                Err(error.localizedDescription().to_string())
            } else if let Some(value) = value.as_ref().and_then(|v| v.downcast_ref::<NSString>()) {
                Ok(value.to_string())
            } else {
                Err("WebKit returned no page snapshot".into())
            }
        };
        on_done(result);
    });
    // A named content world keeps page-replaced JS globals out of the extractor.
    // The DOM is still page-controlled; this is not Chromium's renderer snapshot API.
    // SAFETY: the webview and content world are live UI-thread objects; WebKit copies the block.
    unsafe {
        let world = WKContentWorld::worldWithName(&NSString::from_str("ai-browser.context"), mtm);
        webview
            .webview()
            .evaluateJavaScript_inFrame_inContentWorld_completionHandler(
                &NSString::from_str(script),
                None,
                &world,
                Some(&completion),
            );
    }
}

pub fn show_window(window: &Window) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    app.unhide(None);
    window.set_minimized(false);
    // AppKit frame and screen rectangles share point coordinates across mixed-DPI displays.
    let native = unsafe { &*(window.ns_window() as *const NSWindow) };
    let frame = native.frame();
    let screens = NSScreen::screens(mtm);
    let reachable = screens.iter().any(|screen| {
        let visible = screen.visibleFrame();
        let right = (frame.origin.x + frame.size.width).min(visible.origin.x + visible.size.width);
        let left = frame.origin.x.max(visible.origin.x);
        let top = frame.origin.y + frame.size.height;
        let bottom = (top - 36.0).max(visible.origin.y);
        right - left >= 96.0 && top.min(visible.origin.y + visible.size.height) - bottom >= 18.0
    });
    if !reachable {
        if let Some(screen) = screens.firstObject() {
            let visible = screen.visibleFrame();
            let mut restored = frame;
            restored.size.width = restored.size.width.min(visible.size.width);
            restored.size.height = restored.size.height.min(visible.size.height);
            restored.origin.x = visible.origin.x + (visible.size.width - restored.size.width) / 2.0;
            restored.origin.y = visible.origin.y + visible.size.height - restored.size.height;
            // Tao queues geometry updates; restore the whole frame synchronously on the main thread.
            native.setFrame_display(restored, true);
        }
    }
    window.set_visible(true);
    window.set_focus();
}

pub struct EventMonitor(Retained<AnyObject>);

impl Drop for EventMonitor {
    fn drop(&mut self) {
        // SAFETY: this token was returned by addLocalMonitor and is removed exactly once.
        unsafe { NSEvent::removeMonitor(&self.0) };
    }
}

pub fn install_events(
    window: &Window,
    proxy: EventLoopProxy<AppMessage>,
) -> Result<EventMonitor, String> {
    let mtm = MainThreadMarker::new().ok_or("native events require the main thread")?;
    install_menu(mtm);
    let native_window = window.ns_window() as usize;
    let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: AppKit supplies a live event for the duration of this callback.
        let e = unsafe { event.as_ref() };
        let Some(mtm) = MainThreadMarker::new() else {
            return event.as_ptr();
        };
        let Some(window) = e.window(mtm) else {
            return event.as_ptr();
        };
        if Retained::as_ptr(&window) as usize != native_window {
            return event.as_ptr();
        }
        if e.r#type() == NSEventType::LeftMouseDown {
            let point = e.locationInWindow();
            if let Some(content) = window.contentView() {
                let height = content.bounds().size.height;
                let _ = proxy.send_event(AppMessage::CanvasPointer(
                    point.x as i32,
                    (height - point.y) as i32,
                ));
            }
        } else if e.r#type() == NSEventType::KeyDown {
            let flags = e.modifierFlags();
            let chars = e
                .charactersIgnoringModifiers()
                .map(|s| s.to_string())
                .unwrap_or_default();
            let key = keys::macos_key(e.keyCode(), &chars);
            if let Outcome::Consume(shortcut) = keys::resolve_macos(
                key,
                flags.contains(NSEventModifierFlags::Command),
                flags.contains(NSEventModifierFlags::Control),
                flags.contains(NSEventModifierFlags::Option),
                flags.contains(NSEventModifierFlags::Shift),
            ) {
                if proxy
                    .send_event(AppMessage::NativeShortcut(shortcut))
                    .is_ok()
                {
                    return std::ptr::null_mut();
                }
            }
        }
        event.as_ptr()
    });
    // SAFETY: the block returns the supplied live event or null, and AppKit retains it.
    unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            NSEventMask::KeyDown | NSEventMask::LeftMouseDown,
            &handler,
        )
    }
    .map(EventMonitor)
    .ok_or_else(|| "could not install native keyboard/mouse handling".into())
}

fn install_menu(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    let main = NSMenu::new(mtm);
    for (title, items) in [
        (
            "AI Browser",
            vec![
                ("Hide AI Browser", sel!(hide:), "h"),
                ("Quit AI Browser", sel!(terminate:), "q"),
            ],
        ),
        (
            "Edit",
            vec![
                ("Undo", sel!(undo:), "z"),
                ("Redo", sel!(redo:), "Z"),
                ("Cut", sel!(cut:), "x"),
                ("Copy", sel!(copy:), "c"),
                ("Paste", sel!(paste:), "v"),
                ("Select All", sel!(selectAll:), "a"),
            ],
        ),
    ] {
        let parent = NSMenuItem::new(mtm);
        parent.setTitle(&NSString::from_str(title));
        let menu = NSMenu::new(mtm);
        menu.setTitle(&NSString::from_str(title));
        for (label, action, key) in items {
            // SAFETY: selectors target AppKit's responder chain, not a custom object.
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(label),
                    Some(action),
                    &NSString::from_str(&key.to_ascii_lowercase()),
                )
            };
            item.setKeyEquivalentModifierMask(if key == "Z" {
                NSEventModifierFlags::Command | NSEventModifierFlags::Shift
            } else {
                NSEventModifierFlags::Command
            });
            menu.addItem(&item);
        }

        parent.setSubmenu(Some(&menu));
        main.addItem(&parent);
    }
    app.setMainMenu(Some(&main));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_identifiers_persist_and_isolate_profiles() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let id = create_shared_environment(first.path()).unwrap();
        assert_eq!(create_shared_environment(first.path()).unwrap(), id);
        assert_ne!(create_shared_environment(second.path()).unwrap(), id);
        std::fs::write(first.path().join("profile-id"), b"broken").unwrap();
        assert!(create_shared_environment(first.path()).is_err());
    }

    #[test]
    fn concurrent_first_launches_share_one_profile() {
        let dir = tempfile::tempdir().unwrap();
        let ready = std::sync::Barrier::new(16);
        let ids = std::thread::scope(|scope| {
            let threads = (0..16)
                .map(|_| {
                    scope.spawn(|| {
                        ready.wait();
                        create_shared_environment(dir.path()).unwrap()
                    })
                })
                .collect::<Vec<_>>();
            threads
                .into_iter()
                .map(|t| t.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(ids.iter().all(|id| *id == ids[0]));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn downloads_use_the_configured_directory_and_avoid_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("chosen");
        let suggested = Path::new("/server/chosen/report.pdf");
        let first = download_destination(suggested, &destination).unwrap();
        assert_eq!(first, destination.join("report.pdf"));
        std::fs::write(&first, b"existing").unwrap();
        assert_eq!(
            download_destination(suggested, &destination).unwrap(),
            destination.join("report (1).pdf")
        );
        assert_eq!(
            download_destination(Path::new("../a:b.pdf"), &destination).unwrap(),
            destination.join("a_b.pdf")
        );
    }
}
