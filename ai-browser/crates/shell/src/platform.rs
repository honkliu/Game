//! The browser engine and native host boundary.

use std::path::{Path, PathBuf};
use tao::window::Window;
use wry::{WebView, WebViewBuilder};

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(windows)]
pub use crate::webview2::{
    create_shared_environment, ensure_runtime, environments_constructed, show_fatal,
};
#[cfg(windows)]
pub type Environment = webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Environment;

#[cfg(target_os = "macos")]
pub use macos::{
    create_shared_environment, ensure_runtime, environments_constructed, show_fatal, Environment,
};

pub const ENGINE_NAME: &str = if cfg!(target_os = "macos") {
    "WebKit"
} else {
    "WebView2"
};

pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("AI_BROWSER_DATA_DIR") {
        return PathBuf::from(dir);
    }
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
    });
    base.unwrap_or_else(std::env::temp_dir)
        .join("native-browser")
}

pub fn context_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(if cfg!(target_os = "macos") {
        "webkit"
    } else {
        "webview2"
    })
}

pub fn page_asset_url(url: &str) -> String {
    // Wry rewrites custom schemes during construction, but not in load_url.
    #[cfg(windows)]
    if let Some(path) = url.strip_prefix("asset://shell/") {
        return format!("http://asset.shell/{path}");
    }
    url.to_string()
}

pub fn configure<'a>(
    builder: WebViewBuilder<'a>,
    environment: Option<&Environment>,
    trusted_ui: bool,
) -> WebViewBuilder<'a> {
    #[cfg(windows)]
    {
        use wry::WebViewBuilderExtWindows;
        let mut builder = if trusted_ui {
            builder.with_https_scheme(true)
        } else {
            builder.with_browser_accelerator_keys(false)
        };
        if let Some(env) = environment {
            builder = builder.with_environment(env.clone());
        }
        builder
    }
    #[cfg(target_os = "macos")]
    {
        use wry::WebViewBuilderExtDarwin;
        let _ = trusted_ui;
        let mut builder = builder.with_allow_link_preview(false);
        if let Some(env) = environment {
            builder = builder.with_data_store_identifier(*env);
        }
        builder
    }
}

pub fn shape_webview(webview: &WebView, window: &Window, rect: (i32, i32, i32, i32), radius: i32) {
    #[cfg(windows)]
    {
        use wry::WebViewExtWindows;
        let _ = window;
        crate::sys::round_webview(webview.hwnd().0 as isize, rect.2, rect.3, radius);
    }

    #[cfg(target_os = "macos")]
    {
        let _ = rect;
        macos::shape_webview(webview, f64::from(radius) / window.scale_factor());
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn page_asset_navigation_preserves_state_and_external_urls() {
        let url = "asset://shell/pages/settings.html#state=abc-_";
        #[cfg(windows)]
        assert_eq!(super::page_asset_url(url), "http://asset.shell/pages/settings.html#state=abc-_");
        #[cfg(not(windows))]
        assert_eq!(super::page_asset_url(url), url);
        assert_eq!(super::page_asset_url("https://example.com/asset://shell/"), "https://example.com/asset://shell/");
        assert_eq!(super::page_asset_url("asset://untrusted/page"), "asset://untrusted/page");
    }
}
