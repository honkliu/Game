# macOS

AI Browser uses the same Rust application, AI engine, Preact pane, session format,
approval policy and agent tools on both platforms. Windows uses WebView2; macOS
uses the system WKWebView through the existing Wry/Tao stack. No Chromium runtime
is bundled on macOS.

## Requirements and running

- macOS 14 (Sonoma) or newer, on Apple Silicon or Intel.
- Rust stable, Node.js 24 and Xcode command line tools.

From the repository root on a Mac:

```sh
bash run-ai-browser.sh
```

Or, from `ai-browser`:

```sh
export MACOSX_DEPLOYMENT_TARGET=14.0
npm --prefix ui ci
cargo run --release
```

The UI build is embedded in the executable, just as on Windows. The first Cargo
build runs the UI build automatically. `AI_BROWSER_SKIP_UI_BUILD=1` requires
already-built pane assets.

Settings, history, AI configuration and the session live in
`~/Library/Application Support/native-browser`. `AI_BROWSER_DATA_DIR` overrides
that exact directory on either platform, including for isolated test runs.
`webkit/profile-id` identifies the persistent WKWebsiteDataStore shared by the
window's webviews. WebKit manages the website data itself; it is not stored inside
that directory. Keep the identifier when moving a profile on the same Mac.

## Platform integration

`crates/shell/src/platform.rs` configures the engine; `platform/macos.rs` owns
AppKit input, native menus, profile identifiers, rounded canvas views and WebKit
context capture. `sys/macos.rs` supplies native file opening and process metrics.
The WebView2/Win32 dependencies are Windows-only.

The Mac adapter includes Command shortcuts, native editing menu accelerators,
Control+Tab and Command+Option+arrows for tab switching, Command+brackets for
history, and the same sidebar/canvas/PDF interfaces. Downloads select their real
destination at start because Wry's Mac completion callback omits its path.
PDF viewer addresses use `pdf://localhost/`, not Windows' mapped HTTP origin.
Both platforms bundle Mozilla's PDF.js 6.5.438 **legacy** distribution, which
includes compatibility polyfills needed by older supported WebKit versions.
The vendored `pdf.min.mjs` and `pdf.worker.min.mjs` come unchanged from
`pdfjs-dist@6.5.438` under `legacy/build/`; the upstream license is retained in
`crates/shell/ui/pdf/LICENSE-pdfjs.txt`.

Page extraction runs in a named WKContentWorld, separate from the page's JS
globals, and normalizes visible text, links and document-space rectangles into
the shared citation parser. It reads the top-level DOM, reports unread frames,
and bounds/tracks omitted content. **It is not Chromium's
`DOMSnapshot.captureSnapshot`: identical rendering, site compatibility and
renderer-snapshot provenance are not promised.** Shadow roots are not
traversed by the Mac DOM extractor. Agent context and approved page interactions
continue through the shared probe/tool paths.

The existing PDF-fetch limitations (including authenticated PDFs whose cookies
are not forwarded) remain. The Windows `--proof` budget harness is rejected on
Mac instead of reporting misleading WebView2 process counts; `--self-test` is
available on both platforms. macOS process-tree RSS does not necessarily include
WebKit's separately managed XPC services.

## Verification

```sh
cargo test --locked --workspace
npm --prefix ui run typecheck
npm --prefix ui test
```

The workspace includes a main-thread native `webkit` test executable on macOS.
It creates real child WKWebViews and checks bundled shell/sidebar loading,
isolated-world extraction even when the page replaces `JSON.stringify`,
hidden-content filtering, links, DOM interactions, selectable local PDF text and
a real WKDownload saved into the selected directory.
It uses temporary files and nonpersistent test webviews.

The GitHub Actions workflow runs tests and packages separate Apple Silicon and
Intel apps on macOS runners, alongside Windows regression tests. The existing
CDP E2E harness is Windows-only; `e2e/suite11.mjs` additionally checks the Mac
extraction algorithm against a real Chromium DOM/layout. It does not substitute
for the native WKWebView test.

The implementation was cross-checked from Windows; native Mac execution still
needs to be confirmed on the macOS runners or a local Mac. Cross-compilation
checks do not validate AppKit event delivery, WebKit rendering or native linking.

## Packaging

From `ai-browser` on a Mac:

```sh
bash tools/package-macos.sh
```

This produces `dist/<target>/AI Browser.app` and a ZIP archive. Pass
`aarch64-apple-darwin` or `x86_64-apple-darwin` as the first argument to select an
installed Rust target; the default is the build machine's native target.
The script ad-hoc signs local builds.

For distribution, set `APPLE_SIGNING_IDENTITY` to an installed **Developer ID
Application** identity. To notarize and staple as well, set `APPLE_NOTARY_PROFILE`
to a preconfigured `notarytool` keychain profile. Credentials are not stored in
the repository. CI artifacts are ad-hoc signed, not notarized release downloads.
