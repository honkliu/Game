# AI-native browser: status

Spec: `../ai-native-browser-design-and-engineering-backlog.md`.

## Architecture
- `crates/shell`: the browser and the only executable (`browser.exe` on Windows, `browser` on macOS). Rust, WebView2/WKWebView via wry/tao. It owns identity, tabs, policy and action dispatch. The AI engine (`crates/utility`, library `ai-engine`) runs as a thread inside the same process.
- `crates/contract`: the shared message contract.
- `ui/`: the docked AI pane, TypeScript + Preact + signals, built with Vite into `crates/shell/ui/sidebar/` and **embedded in `browser.exe`** by `crates/shell/build.rs` (which runs `npm run build`; set `AI_BROWSER_SKIP_UI_BUILD=1` to skip, `AI_BROWSER_UI_DIR=<dir>` to serve shell assets from disk).
- Kernel: WebView2 on Windows; system WKWebView on macOS 14+. The AI engine and UI
  are shared. Platform adapters, build instructions and verification boundaries:
  [macOS support](docs/MACOS.md).

## AI pane (Edge-Copilot style)
- **Agent** switch in the header (not tabs). Off: cited answers about the page. On: the tool-calling agent (browse, read, click, type, fetch, search, skills, MCP) with approvals.
- **Auto/Manual** switch (agent mode): Auto navigates the page itself (no approval, opens in the current view); Manual only shows a link card to click.
- The AI pane is drag-resizable from its left edge (min 320px, no max beyond leaving a page sliver); width is not persisted.
- Context bar under the transcript: page link, content size, live "page updated" indicator, tabs, and the **selected text card** with Explain / Verify / Translate / Rewrite. "View / edit context" opens the exact context the model gets; you can edit it. An edit is a temporary overlay: when the page content changes the context is regenerated and the edit is discarded. Skills, system prompt, instructions and MCP are stored and persist.
- **Settings** is one reusable browser tab with internal sections: General, Models, Tools, Skills, MCP, Prompts, Instructions, History, Downloads and Saved chats. Both the browser menu and River's gear open it; History/Downloads shortcuts select sections of that same tab. River remains a chat pane and keeps its draft while Settings is open. Saved chats reopen in River, including when its pane has not yet been created.
- **Models** needs only a model name and an OpenAI-compatible endpoint; an API key is optional. Full `/chat/completions` URLs are accepted. Connection IDs and additional model metadata are under Advanced. Multiple endpoints and per-endpoint models remain supported, with separate vault-backed credentials.
- Type **`/model`** in either chat mode for a searchable keyboard-accessible picker, **`/model name`** or **`/model connection/model`** to switch, and **`/model default`** to reset. Commands run locally, without adding conversation turns, calling an endpoint or changing routing configuration. Selection persists across pane reloads and switching preserves the conversation. Ambiguous or unavailable models are not silently selected.
- **Tools** lists browser/local operations, configured HTTP requests and individual MCP tools. Every tool has **Edit** for its model-visible name, title, description and parameter schema; HTTP tools also expose method, URL, JSON body and response format. MCP tools link to their server connection editor. Built-in/local/MCP definitions can be restored to their source without changing enablement. Stable handler IDs preserve dispatch and approvals when model-visible names change. Edits are saved in the profile's `agent.json`.
- Tools are supplied through the model request's **`tools`** field with `tool_choice: "auto"`, separately from the System Prompt. The model returns `tool_calls`; the browser dispatches them to the shown implementation/endpoint and returns results to the model. Each Tools card shows the actual function definition. Renamed tools get a short alias mapping alongside the rendered system instructions so older prompt/skill references remain understandable. There is no need to duplicate function schemas in System Prompt.
- The model registry comes only from the saved selection; there is no additional execution-time configuration check. Existing profiles migrate their current tools into visible configuration once. New unconfigured tools are not implicitly enabled. Editing a schema does not rewrite a native operation or an MCP server's implementation: parameter names must still match the selected handler.
- To replace the agent's DuckDuckGo search with MoonShot: River/洛 > Settings > Tools > Edit `web_search` > **MoonShot search template**, enter your `/v1/search` endpoint and Save. The template uses POST JSON with `mode`, `query`, `offset`, `limit`. Add a **Document lookup template** tool pointed at `/v1/documents/{{document_id}}` for full content. Configuring a tool does not contact its endpoint. The address-bar search template is separate.

## Bidirectional interaction
- Browser to AI: page content, form field values (passwords hidden), selection, tabs. The page probe reports DOM mutations, typing, clicks and in-page route changes, so the context changes without a URL change.
- AI to browser: navigate, open/activate/close tabs, click, type, scroll, read other tabs. Writes need approval unless auto-approve is on.
- Selecting text on a page: the toolbar (pane closed) or the pane's selection card (pane open); both route into the pane in the current mode.

## Canvas tab
- A canvas is one tab holding a grid of live pages (1–4 columns × 1–3 rows, default 3×2, so at most 6 by default and 12 at the extreme). Rail "New canvas" button, `new-canvas` verb, or the agent's `canvas` tool.
- Each cell is a real page with its own history and is content only: no header, no address box, no border (cells are 2 px apart; only the focused cell shows a ring). Click anywhere in the canvas to focus the nearest cell. The shared address bar, find, zoom (toolbar and Ctrl+wheel) and the AI context follow the focused cell. A shared canvas bar reshapes the grid (extra pages close). Links opening new windows fill the next empty cell. The agent targets cells by 0-based `row`/`col`. The grid is saved with the session.

## PDF viewer
- PDF links and addresses (`*.pdf`) open in a bundled pdf.js viewer (`pdf://` protocol, `ui/pdf/`) instead of WebView2's plugin viewer, so selected text and page text reach the AI like any page. The tab keeps the real PDF URL. Native fetching uses the page's browser cookies, never forwards them to another origin, and limits loads to 64 MiB with cancellation and deadlines. Loading errors are visible. Windows cookie-authenticated loading, redirect isolation, limits, text selection and PDF find are covered by `e2e/suite5.mjs`. macOS parent-domain cookie handling still needs native validation.

## Window chrome (Chrome layout)
- Frameless window: a 36 px tab strip is the title bar (tabs, `+`, new canvas, drag area, min/max/close), then one 36 px address row (back/forward/reload, omnibox, zoom − % +, River/洛, a plain page-search box, Downloads, Settings), then the page. The assistant retains its ✦ icon; the displayed name follows the browser language. Horizontal tabs are the default; Settings `vertical_tabs` switches to the old rail. Settings opens its reusable tab directly, without a dropdown. Ctrl+K remains the keyboard shortcut for the command palette. Ctrl+F focuses the always-visible search box; typing highlights literal, case-insensitive matches, Enter moves to the next match and Escape clears. There is no separate Find row or advanced search controls, and searching preserves the River pane. Resize handles are on the top edge and corners; double-click the drag area to maximize.
- Chat/agent replies use Marked for Markdown parsing and DOMPurify for HTML sanitization, rather than a handwritten parser. Tables, lists, code, inline HTML such as superscript notes, and standard/reference/bare links are supported. Link clicks open native browser tabs without navigating River. Text selection is reported only after the mouse is released. Markdown rendering and native link navigation are covered by `ui/tests/core.test.tsx` and `e2e/suite12.mjs`.
- Opening a shell menu temporarily hides native page and River views without closing the pane or losing drafts. Dismissal restores them without grabbing focus from the working page. Windows custom-protocol aliases consistently use HTTPS, avoiding automatic HTTP upgrades that previously broke Settings reloads after opening River.

## Verified (automated)
- `cargo test --workspace` (all pass), `npm test` in `ui` (Vitest), Playwright e2e over CDP: `e2e/suite.mjs` … `suite5.mjs` (shell, ask, multi-tab, agent loop, approvals, prompts, skills, MCP, live context, editable context, selection, context panel, history, canvas: layout, focus, real-click focus, zoom, reshape, limits, clear, pop-ups, close, agent tool; PDF viewer; canvas restore after restart checked by hand script). The real-click tests can fail if another window steals the foreground while they run.
- Run e2e: build into `%TEMP%\ai-target`, `set BROWSER_EXE`, then `node e2e\suiteN.mjs`.
- `e2e/suite12.mjs` exercises direct toolbar-to-Settings opening, including native hit-testing with River open, preserved drafts/forms, focus after layout changes, save/reload and failed-save reporting, startup/existing-view new tabs without duplicate search controls, local-history inline completion and explicit selection, configured search, all-six-cell zoom/return, edited warm tabs, inline page search without layout changes or closing River, and hidden/reloaded AI transcript recovery. The focused fixture-only regressions run with `npm --prefix e2e ci` then `npm --prefix e2e test` after setting `BROWSER_EXE`; Windows CI runs the same command.
- `e2e/suite13.mjs` covers River/洛 labels and the retained icon, the thin active-tab bottom line, visible tool selection, HTTP search/document requests and persistence. It also checks edited function definitions and calls through renamed browser, local, HTTP and MCP tools, including unchanged manual-navigation behavior.
- `e2e/suite14.mjs` covers minimal keyless sample-model setup, advanced multi-model editing, failed-save preservation, local `/model` commands and real switching in both chat modes, credential removal, the single Settings tab, saved-chat reopening, browsing-history deletion and download-list actions.

## Not verified or not done
- Native macOS runtime validation is pending. The WKWebView backend and native smoke test cross-check from Windows; the workflow is configured to run and package on Apple Silicon and Intel Macs.
- The real Qwen endpoint was exercised only through earlier manual runs, not in e2e. MCP stdio on Windows and real MCP servers, click/fill on arbitrary real sites, and waking sleeping tabs are untested.
- Submitted searches use the configurable `search_template` (DuckDuckGo HTML by default); address completion queries local history, not a network suggestion service.
- Provider keys use the Windows/macOS credential vault, with legacy `ai.key` migration. Native vault migration and restart behavior still need runtime validation. MCP tokens and conversation history remain on disk; their privacy/retention improvements are not complete.
- Not built: anchored annotations (§2.1), result strip (§2.4), Chromium fork, accessibility tree.

## Run
`cd ai-browser; cargo run` (needs Node for the first build). Endpoints, models and keys: Settings > Models. Switch with `/model` in River/洛.

On macOS, run `bash run-ai-browser.sh` from the repository root. Native app
packaging: `bash tools/package-macos.sh` from `ai-browser`.

## PDF cells
- Extensionless PDF URLs (e.g. arxiv.org/pdf/ID) are detected via contentType and routed to the bundled viewer.
- Zoom (address-bar buttons, shortcuts, Ctrl+wheel) in a viewer tab goes through window.__pdfZoom (e2e/suite8.mjs).


## Split view, tab colours, PDF toolbar
- Canvas panes are Edge-style: 4 px apart, small rounded corners (window region clip), blue ring on the active pane, draggable dividers (double-click resets), min 12% share.
- New-canvas button opens a layout picker (2x1 ... 4x3); canvas tab right-click offers the same layouts.
- Tab headers retain their per-tab hues. A thin, continuous 3 px bottom line matches the active header, including in dark mode and when a canvas tab is shown. Empty strip space stays neutral; inactive headers stay distinct. Page content is never tinted.
- Local file:// PDFs feed selections to the AI; the PDF viewer has a toolbar (name, page n/N, zoom -/+/Fit) wired to the shared page zoom.
- Real-OS-input e2e tests need an unlocked, foreground desktop.

## Close popups (page level)

- Shell command `page.dismiss_popups {scope: cell|tab|all, cell?, row?, col?, tab_id?}` and agent tool `dismiss_popups` (no approval needed). `cell` = focused cell (or the one named), `tab` = every cell of the current canvas, `all` = every open page.
- `app/dismiss.js` finds fixed/high-z elements that are dialogs, backdrops, ads (ad iframes or ad/popup class names), or cookie banners, clicks their own close control, hides what is left and restores page scrolling. Headers, nav bars and the main content are never touched. Heuristic, top frame only: popups inside cross-origin iframes, or that appear later, need another run.
- Tested in `e2e/suite10.mjs` (2x2 canvas, command and agent tool, header survives).

