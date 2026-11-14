# Revised AI-native browser design and engineering backlog

## Executive direction

Build a **thin Chromium fork with a browser-native AI platform**. Do not develop a new rendering engine.

The product should be optimized in this order:

1. **User efficiency**
2. **Safe, predictable action**
3. **Model choice**
4. **Compatibility**
5. **Privacy/security specialization**

Security is not the primary marketing differentiator, but it remains a release gate because an AI browser can read and act across authenticated websites.

The revised design corrects several weaknesses in the earlier proposal:

- AI is no longer modeled mainly as a sidebar.
- Every conversation is explicitly bound to page and workspace versions.
- The browser process—not a webpage or model—owns navigation and context identity.
- AI computation runs outside the browser UI process.
- Inline answers are browser overlays, not modifications to website DOM.
- Observation and action use separate capability contracts.
- Cancellation, stale-context handling, bounded output, and telemetry are first-class.
- The initial product is read-only and useful before autonomous actions are introduced.

This aligns with the “hidden, not headless,” conversation-owned, typed-degradation approach described in Autopilot Local Browser Phase 0 - Look with me.md. It also addresses the key finding in Edge Chromium Browser Use Capability Surface.md: existing browser tooling can be a strong actuator but should not be the authority for identity, permissions, policy, or task lifecycle.

---

# 1. Revised product concept

The product is an **AI workspace browser**, not “Chrome plus a chatbot.”

A workspace consists of:

```text
Workspace
├── AI thread
├── Active browsing pane
├── Optional comparison/preview pane
├── Managed candidate tabs
├── User annotations
├── Context history
└── Task state
```

The browser must support four interaction modes:

| Mode | Purpose | Example |
|---|---|---|
| **Ask here** | Ask about selected or visible content | “What does this paragraph mean?” |
| **Ask this page** | Understand the active document | “Summarize the article and identify unsupported claims.” |
| **Ask this workspace** | Reason across selected tabs | “Compare these four products.” |
| **Do this** | Perform controlled browser actions | “Open the three best candidates beside this page.” |

The user should never need to wonder:

- which page the AI read;
- whether it followed a link;
- whether it is referring to the old or new tab;
- which model is in use;
- whether it is only recommending an action or actually performing it.

---

# 2. Revised user experience

## 2.1 Inline selection interaction

When the user selects text, display a browser-owned floating toolbar:

```text
Ask  |  Explain  |  Verify  |  Translate  |  Rewrite  |  More
```

Selecting **Ask** opens a compact composer beside the selection. The streamed response appears as a Word-style comment attached to the passage.

The comment supports:

- follow-up questions;
- citations back to source passages;
- pinning;
- collapse/expand;
- copy;
- convert to workspace note;
- open cited links beside the current page;
- reattachment after reload;
- “outdated” indication if source text changes.

The comment UI must be rendered in a privileged overlay or isolated browser surface. It must not be a normal element inserted into page DOM, because page JavaScript and CSS could interfere with or impersonate it.

### Annotation anchor

```ts
interface AnnotationAnchor {
  documentKey: DocumentKey;
  exactQuote: string;
  prefix: string;
  suffix: string;
  textPosition?: { start: number; end: number };
  semanticBlockId?: string;
  accessibilityPath?: string[];
  domPathHint?: string;
  originalRect?: Rect;
  fingerprint: string;
}
```

Reattachment order:

1. semantic block ID;
2. quote plus prefix/suffix;
3. normalized text position;
4. accessibility path;
5. fuzzy quote match;
6. detach and ask the user to re-anchor.

Using multiple selectors is important because DOM paths alone break on common page updates. This follows the target-and-selector approach standardized by the Web Annotation Data Model.

---

## 2.2 Context header

Every AI surface should display an explicit context header:

```text
Using:
✓ Active tab: “Article title”
✓ Current selection
✓ 2 related tabs
△ Embedded video has no transcript
Model: Local Fast ▼
```

The user can remove or add context without rewriting the prompt.

Context selection should support:

- active page;
- current selection;
- visible viewport;
- entire page;
- chosen tabs;
- all managed tabs in a workspace;
- uploaded files;
- previous workspace notes.

The default behavior should be intelligent but visible. Automatic context must never be invisible.

---

## 2.3 Tab and navigation behavior

The browser process assigns identity to every document:

```ts
interface DocumentKey {
  profileId: string;
  workspaceId: string;
  tabId: number;
  frameTreeNodeId: number;
  documentToken: string;
  navigationSequence: number;
}
```

A URL is metadata—not identity.

The following events create or update context:

```text
TabActivated
NavigationStarted
NavigationRedirected
DocumentCommitted
HistoryStateChanged
FrameAttached
FrameDetached
MeaningfulContentChanged
SelectionChanged
ViewportChanged
DocumentDestroyed
```

On a tab switch:

1. Update the context header immediately.
2. Load the last snapshot for that tab.
3. Refresh it in the background.
4. Preserve the previous tab’s thread association.
5. Prevent old responses from acting on the new tab.

On navigation:

1. Freeze the previous snapshot as historical.
2. increment `navigationSequence`;
3. invalidate element handles and action targets;
4. capture initial destination metadata;
5. capture the destination semantic snapshot;
6. update the conversation binding;
7. allow actions only after the new snapshot is ready.

Chromium navigation can cross documents, renderers, and security contexts, so this lifecycle must use browser-process navigation events rather than renderer URL polling.

---

## 2.4 Split results and automatic opening

AI output should produce typed browser artifacts, not only Markdown:

```ts
type AnswerArtifact =
  | PassageCitation
  | BrowserLink
  | ComparisonTable
  | ProductCandidate
  | WorkspaceNote
  | ProposedAction
  | CompletedAction;
```

A link artifact includes:

```ts
interface BrowserLink {
  artifactId: string;
  url: string;
  displayTitle: string;
  provenance: "page" | "search" | "tool" | "model-suggested";
  relationship: "citation" | "candidate" | "original-source" | "related";
  recommendedPresentation:
    | "preview"
    | "split-pane"
    | "background-tab"
    | "current-tab";
}
```

For a request such as “find me the best shoes on Amazon”:

1. The AI extracts constraints.
2. Search opens in a managed tab.
3. Candidate product pages open in managed background tabs.
4. The browser extracts comparable properties.
5. A comparison table appears in the assistant pane.
6. The highest-ranked candidate automatically opens in the browsing pane.
7. Other candidates appear in a result strip.
8. The user can say “show the second one” without clicking or reopening anything.

Automatic opening should apply only to safe navigation and previews. Adding to cart, submitting forms, downloading executables, signing in, or purchasing requires stronger user involvement.

---

## 2.5 Browser-native omnibox

The omnibox accepts URLs, searches, and AI tasks:

```text
example.com
running shoes under $150
compare these tabs
summarize this page
find the original source
open the top result beside this page
```

Do not build a parallel JavaScript omnibox. Reuse Chromium classification, autocomplete, keyboard, accessibility, IME, policy, and search-provider behavior.

A useful implementation pattern is documented in New Tab for Copilot In-App Browsing Dev Spec.docx: WebUI can own surrounding chrome and geometry, while a native Chromium control owns editing, suggestions, acceptance, and navigation.

---

# 3. Revised technical architecture

```text
┌──────────────────────────────────────────────────────────────┐
│                 Native Browser Experience                    │
│ Omnibox | Tabs | Inline comments | Workspace | Split panes   │
└────────────────────────────┬─────────────────────────────────┘
                             │ typed Mojo interfaces
┌────────────────────────────▼─────────────────────────────────┐
│                    Browser AI Coordinator                    │
│ Context identity | Workspace binding | Lifecycle | Policy    │
└──────────────┬───────────────────┬──────────────────┬────────┘
               │                   │                  │
      ┌────────▼────────┐ ┌────────▼────────┐ ┌──────▼────────┐
      │ Context Service │ │ Action Runtime  │ │ Overlay Host  │
      │ snapshots/diffs │ │ plan/act/verify │ │ annotations   │
      └────────┬────────┘ └────────┬────────┘ └───────────────┘
               │                   │
        renderer probes      browser capabilities
               │                   │
┌──────────────▼───────────────────▼───────────────────────────┐
│                      Chromium Platform                       │
│ WebContents | Navigation | AX tree | Blink | tabs | network │
└────────────────────────────┬─────────────────────────────────┘
                             │ isolated IPC
┌────────────────────────────▼─────────────────────────────────┐
│                     AI Utility Process                       │
│ Model broker | Router | Orchestrator | Context compaction    │
└───────────┬──────────────────┬─────────────────────┬─────────┘
            │                  │                     │
       Local models       Cloud providers       Tool/MCP adapters
```

Chromium’s multi-process architecture provides the correct foundation: site renderers remain separated from privileged browser functionality, and faults can be isolated rather than placing all page and AI code in one process.

## Process ownership

### Browser process

Owns:

- tab and document identity;
- navigation lifecycle;
- workspace membership;
- policy and permissions;
- action validation;
- native UI;
- annotation placement;
- context freshness;
- final action dispatch.

Must not run:

- cloud model HTTP implementations on the UI thread;
- long model inference;
- vector indexing;
- arbitrary model-generated code.

### AI utility process

Owns:

- provider adapters;
- prompt construction;
- model routing;
- conversation reduction;
- structured-output validation;
- task planning;
- local model execution where practical.

It has no direct authority to manipulate tabs or pages. It can only request typed browser capabilities.

### Renderer process

Owns narrowly scoped probes for:

- semantic DOM extraction;
- text blocks;
- selection;
- page-specific metadata;
- visible form/control descriptors;
- mutation summaries.

It must not return cookies, password values, authentication headers, browser storage, or hidden credential fields.

### Browser UI/compositor

Owns:

- inline comments;
- selection toolbar;
- action indicator;
- split panes;
- AI context header;
- approval surfaces.

---

# 4. Context Engine

## 4.1 Snapshot model

```ts
interface PageSnapshot {
  key: DocumentKey;
  snapshotVersion: number;
  capturedAt: number;

  metadata: {
    url: string;
    origin: string;
    canonicalUrl?: string;
    title: string;
    language?: string;
    pageType: PageType;
  };

  blocks: SemanticBlock[];
  links: LinkDescriptor[];
  tables: TableDescriptor[];
  controls: ControlDescriptor[];
  images: ImageDescriptor[];
  transcripts: TranscriptSegment[];
  structuredData: StructuredDataRecord[];

  userState: {
    selection?: AnnotationAnchor;
    viewport: Rect;
    focusedControl?: StableElementRef;
    scrollPosition: number;
  };

  coverage: ContextCoverage;
}
```

### Context coverage

```ts
interface ContextCoverage {
  status: "complete-enough" | "partial" | "updating" | "blocked";
  capturedFrames: number;
  inaccessibleFrames: CoverageGap[];
  unloadedRegions: CoverageGap[];
  unsupportedContent: CoverageGap[];
  hasUntranscribedMedia: boolean;
  estimatedCompleteness: number;
}
```

Do not claim “AI has read the whole page” when the browser cannot access third-party frames, unloaded content, canvas text, or media.

## 4.2 Extraction hierarchy

Use:

1. browser and document metadata;
2. accessibility tree;
3. semantic DOM;
4. rendered visibility/layout;
5. structured data;
6. page-type extractors;
7. PDF/document extractors;
8. transcripts;
9. OCR/vision fallback.

The Chrome DevTools Protocol demonstrates useful structured inspection surfaces for DOM, accessibility, page, target, and browser state, although the product should expose stable internal contracts rather than making CDP its permanent product API.

## 4.3 Incremental updates

A mutation does not automatically justify a full recapture.

```ts
interface SnapshotDelta {
  key: DocumentKey;
  baseVersion: number;
  newVersion: number;
  addedBlocks: SemanticBlock[];
  updatedBlocks: SemanticBlock[];
  removedBlockIds: string[];
  linkChanges: LinkDelta[];
  coverageChanges?: Partial<ContextCoverage>;
}
```

Classify changes:

| Change type | Handling |
|---|---|
| Advertisement animation | Ignore |
| Price or availability | Immediate delta |
| Article body replacement | Rebuild main-content section |
| Dialog appearance | Immediate control-tree update |
| History API route change | New navigation sequence |
| Infinite-scroll append | Append semantic blocks |
| Authentication state change | Revalidate context policy |

---

# 5. Conversation and stale-context model

Every user turn stores immutable context references:

```ts
interface ConversationBinding {
  conversationId: string;
  workspaceId: string;
  contextRefs: Array<{
    documentKey: DocumentKey;
    snapshotVersion: number;
    blockIds?: string[];
  }>;
}
```

Every model response carries:

```ts
interface ResponseBinding {
  requestId: string;
  generatedFrom: ConversationBinding;
  completedAt: number;
  staleStatus:
    | "current"
    | "content-changed"
    | "document-replaced"
    | "tab-closed";
}
```

Rules:

- A stale answer may still be displayed.
- A stale answer must be labeled.
- An action from a stale answer cannot execute without replanning.
- An annotation may render only if its anchor resolves against the current document.
- A navigation action must target a stable tab/document handle, not “the active tab” resolved earlier.

---

# 6. Model platform

## 6.1 Provider contract

```ts
interface ModelProvider {
  providerId: string;
  listModels(): Promise<ModelManifest[]>;
  createSession(config: ModelSessionConfig): Promise<ModelSession>;
}

interface ModelManifest {
  modelId: string;
  modalities: Array<"text" | "image" | "audio">;
  supportsTools: boolean;
  supportsStructuredOutput: boolean;
  supportsStreaming: boolean;
  supportsEmbeddings: boolean;
  execution: "local" | "cloud" | "hybrid";
  contextWindow: number;
  dataPolicyTags: string[];
  costClass: "free" | "low" | "medium" | "high";
  latencyClass: "interactive" | "standard" | "batch";
}
```

## 6.2 Routing policy

```yaml
routes:
  selection_explanation:
    prefer: [local-fast, cloud-fast]
  page_summary:
    prefer: [local-long-context, cloud-standard]
  cross_tab_research:
    prefer: [cloud-reasoning]
  private_page:
    require_tags: [local-execution]
  visual_fallback:
    require_modalities: [image]
```

Users can select:

- Auto
- Local only
- Preferred provider
- Preferred model per task
- Workspace-specific model
- Domain-specific restrictions
- Fallback chain

Provider adapters must normalize:

- streaming;
- tool calls;
- structured output;
- cancellation;
- token accounting;
- retry behavior;
- image input;
- errors;
- rate limits.

---

# 7. Action runtime

## 7.1 Four action levels

| Level | Capability | Examples |
|---|---|---|
| Observe | Read-only | Read page, find passage, inspect links |
| Navigate | Reversible | Open, close, switch, preview, scroll |
| Interact | Page mutation | Type, filter, select, expand |
| Commit | Consequential | Submit, upload, add to cart, purchase |

## 7.2 Execution order

Prefer:

1. browser-native semantic command;
2. page-advertised structured tool;
3. accessibility/DOM locator;
4. visual target;
5. coordinate action only as fallback.

## 7.3 Action contract

```ts
interface BrowserActionRequest {
  actionId: string;
  taskId: string;
  expectedDocument: DocumentKey;
  expectedSnapshotVersion: number;
  action: BrowserAction;
  approvalClass: ApprovalClass;
  deadline: number;
  idempotency: "safe" | "conditional" | "unsafe";
}
```

Before action:

- resolve target again;
- verify document identity;
- verify origin;
- verify snapshot assumptions;
- evaluate policy;
- obtain required approval.

After action:

- collect a new observation;
- verify expected state change;
- return a typed result;
- stop on unexpected navigation or dialog.

The browser-use research identifies cancellation as a contract gap that is inexpensive to solve before broad automation but expensive to retrofit later. It also recommends recording policy decisions without logging clicked coordinates or typed customer content.Edge Chromium Browser Use Capability Surface.md

---

# 8. Data and privacy behavior

Context collection should remain browser-owned.

A remote page may receive safe metadata such as titles or opaque handles, but it should not receive the contents of other tabs. A similar browser-owned capture principle is documented in Contextual AI Mode Search - Security Design Document.docx.

Default storage:

| Data | Default |
|---|---|
| Current snapshot | Memory |
| Snapshot deltas | Bounded memory cache |
| AI conversation | User-configured history |
| Inline comments | Local profile storage |
| Personal memory | Explicit opt-in |
| Raw screenshots | Not persisted |
| Password/cookie/token data | Never collected |
| Action audit | Metadata only |

Required controls:

- clear workspace memory;
- clear all AI data;
- disable cloud models by domain;
- disable automation;
- inspect current AI context;
- inspect active task actions;
- kill switch for every remotely rolled-out capability.

---

# 9. Delivery strategy

The initial milestone should be a useful **read-only vertical slice**, not a broad but unreliable agent.

## Release 0 — Technical proof

- Chromium shell integration
- model-provider abstraction
- active-page extraction
- selection Q&A
- inline overlay
- navigation invalidation
- no page mutations

## Release 1 — “Read with me” private alpha

- page and multi-tab context
- persistent annotations
- model selection
- citations and scroll-to-source
- split previews
- show/hide/stop
- typed failure states
- internal quality dashboards

## Release 2 — “Act with me” beta

- managed tabs
- safe navigation
- filters and reversible page interaction
- action trace
- cancellation
- approvals
- post-action verification

## Release 3 — “Work with me”

- shopping and research workspaces
- automatic candidate opening
- comparison tables
- saved workflows
- external tool ecosystem
- cross-device or unattended work only after separate review

A current internal cycle plan already treats in-app links, tab management, multi-tab context, unified settings, self-host metrics, and release-quality dashboards as related deliverables rather than isolated features.

---

# 10. Prioritized engineering backlog

## Backlog conventions

- **P0:** required for private alpha
- **P1:** required for beta or core differentiation
- **P2:** post-beta expansion
- **S:** up to approximately one engineer-week
- **M:** approximately one to three engineer-weeks
- **L:** approximately three to six engineer-weeks; should be decomposed during planning

These are relative planning estimates, not delivery commitments.

## Epic A — Foundations and contracts

| ID | Pri | Size | Story and acceptance criteria | Depends |
|---|---:|---:|---|---|
| FND-01 | P0 | M | Define `WorkspaceId`, `TabId`, `DocumentKey`, `SnapshotVersion`, `TaskId`, and ownership rules. Identity substitution and URL-as-identity tests pass. | — |
| FND-02 | P0 | M | Define versioned context, action, progress, cancellation, and error schemas. Old clients reject incompatible major versions cleanly. | FND-01 |
| FND-03 | P0 | M | Establish browser/AI/renderer process boundaries and Mojo interfaces. No model/network work blocks the browser UI thread. | FND-01 |
| FND-04 | P0 | S | Create typed failure taxonomy: unavailable, blocked, stale, cancelled, timed out, unsupported, output limited. | FND-02 |
| FND-05 | P0 | M | Establish Chromium upstream-sync pipeline and OWNERS rules. A clean upstream merge can be built and smoke-tested automatically. | — |

## Epic B — Browser shell and workspace

| ID | Pri | Size | Story and acceptance criteria | Depends |
|---|---:|---:|---|---|
| SHL-01 | P0 | L | Implement native AI workspace container with browser and assistant panes. Resize, focus, keyboard, accessibility, localization, and forced colors work. | FND-03 |
| SHL-02 | P0 | L | Implement native omnibox overlay using Chromium providers and ranking. URLs, searches, IME, policy, and keyboard acceptance behave like Chromium. | SHL-01 |
| SHL-03 | P0 | M | Implement conversation/workspace-owned tabs. A tab cannot be accidentally driven by another workspace. | FND-01, SHL-01 |
| SHL-04 | P1 | M | Add split view and preview pane. AI link artifacts open beside the assistant without replacing it. | SHL-01 |
| SHL-05 | P1 | M | Add managed candidate-tab strip with pin, dismiss, reorder, and “show next.” | SHL-03, SHL-04 |

## Epic C — Context Engine

| ID | Pri | Size | Story and acceptance criteria | Depends |
|---|---:|---:|---|---|
| CTX-01 | P0 | L | Implement metadata, semantic-DOM, accessibility, link, and table extraction. Authenticated test pages return bounded structured snapshots. | FND-02 |
| CTX-02 | P0 | M | Implement browser-process navigation lifecycle and document-token tracking. Redirects, reloads, History API changes, and back/forward produce correct identities. | FND-01 |
| CTX-03 | P0 | M | Implement active-tab context switching. P95 context-header update meets the target and no previous-tab response can act on the new tab. | CTX-01, CTX-02 |
| CTX-04 | P0 | L | Implement incremental snapshot deltas and meaningful-mutation classification. Decorative mutation storms do not trigger full extraction. | CTX-01 |
| CTX-05 | P1 | M | Implement context-coverage reporting for frames, lazy content, media, PDFs, and unsupported regions. Partial context is visible to the user and model. | CTX-01 |
| CTX-06 | P1 | L | Add PDF, transcript, structured-data, product, search-result, and article extractors. Each extractor has golden-page tests. | CTX-01, CTX-05 |

## Epic D — Inline interaction and annotations

| ID | Pri | Size | Story and acceptance criteria | Depends |
|---|---:|---:|---|---|
| ANN-01 | P0 | M | Implement browser-owned selection toolbar. It appears quickly and cannot be restyled or intercepted by the page. | SHL-01 |
| ANN-02 | P0 | L | Implement streamed inline answer card with follow-up composer and citations. Page navigation cancels or detaches the card correctly. | ANN-01, MOD-02 |
| ANN-03 | P0 | M | Implement multi-selector annotation anchoring and confidence scoring. Ordinary article edits preserve at least the target reattachment rate. | CTX-01 |
| ANN-04 | P1 | M | Persist pinned comments locally and reattach them after reload. Detached annotations are identified rather than silently misplaced. | ANN-03 |
| ANN-05 | P1 | M | Add scroll-to-source, highlight, collapse, workspace-note conversion, copy, and delete. All operations are keyboard accessible. | ANN-02, ANN-04 |

## Epic E — Model platform

| ID | Pri | Size | Story and acceptance criteria | Depends |
|---|---:|---:|---|---|
| MOD-01 | P0 | M | Implement provider-neutral model manifest and session contracts. Two independently implemented providers pass the same conformance suite. | FND-02 |
| MOD-02 | P0 | L | Implement streaming, structured output, cancellation, retry, and error normalization. Cancelling a request stops token delivery and releases resources. | MOD-01 |
| MOD-03 | P0 | M | Implement task-based model routing with local-only and preferred-provider policies. Routing decisions are inspectable. | MOD-01 |
| MOD-04 | P1 | M | Implement model settings UI for Auto, preferred model, workspace override, local-only, and fallback order. | MOD-03 |
| MOD-05 | P1 | L | Implement context reduction, semantic block selection, deduplication, token budgeting, and cache reuse. Large pages do not require full-DOM prompt submission. | CTX-01, MOD-02 |

## Epic F — Agent and action runtime

| ID | Pri | Size | Story and acceptance criteria | Depends |
|---|---:|---:|---|---|
| ACT-01 | P0 | M | Implement read-only tools: active context, blocks, links, tabs, selection, find, and scroll-to-source. Tools return provenance and document versions. | CTX-01 |
| ACT-02 | P1 | L | Implement stable accessibility/DOM locator format with just-in-time resolution. Locators fail typed rather than clicking approximate elements. | CTX-01 |
| ACT-03 | P1 | M | Implement managed navigation: open, preview, switch, back, forward, reload, and close. Actions verify the expected destination. | SHL-03, ACT-01 |
| ACT-04 | P1 | L | Implement reversible interaction: click, type, select, expand, and filter. Password and payment fields are excluded. | ACT-02, SAF-02 |
| ACT-05 | P1 | M | Implement tab locks and workspace ownership. Concurrent tasks cannot manipulate the same tab without arbitration. | SHL-03 |
| ACT-06 | P0 | M | Implement end-to-end cancellation and deadline handling. Stop abandons action batches, releases locks, and emits one terminal state. | FND-02, ACT-05 |
| ACT-07 | P1 | L | Implement post-action observation and verification. Unexpected origin, dialog, permission, or state transition stops the task. | ACT-03, ACT-04 |

## Epic G — Results and task experiences

| ID | Pri | Size | Story and acceptance criteria | Depends |
|---|---:|---:|---|---|
| WRK-01 | P1 | M | Implement typed link and citation artifacts. Model-generated text cannot directly trigger arbitrary navigation. | MOD-02, ACT-03 |
| WRK-02 | P1 | L | Implement comparison-table artifact backed by extracted fields and source links. Every displayed value retains provenance. | CTX-06, WRK-01 |
| WRK-03 | P1 | L | Implement shopping/research task flow with candidate collection, ranking criteria, managed tabs, and split preview. | SHL-05, WRK-02 |
| WRK-04 | P2 | L | Implement reusable task templates and saved workflows. Replaying a workflow revalidates domains, context, and action policy. | ACT-07, SAF-03 |

## Epic H — Safety, governance, and data handling

| ID | Pri | Size | Story and acceptance criteria | Depends |
|---|---:|---:|---|---|
| SAF-01 | P0 | M | Implement provenance labels for user, page, browser, tool, and model content. Page text can never become control instructions. | CTX-01 |
| SAF-02 | P0 | L | Implement approval classes for observe, navigate, interact, and commit. High-impact actions cannot bypass the approval broker. | FND-02 |
| SAF-03 | P0 | M | Implement URL, scheme, origin, profile, enterprise-policy, and content-sharing checks. Policy failures are typed and fail closed. | SAF-02 |
| SAF-04 | P0 | M | Implement content-free audit and redacted telemetry. Tests confirm no page text, typed values, credentials, cookies, or query fragments enter audit events. | FND-02 |

## Epic I — Quality, observability, and release

| ID | Pri | Size | Story and acceptance criteria | Depends |
|---|---:|---:|---|---|
| QAL-01 | P0 | L | Create deterministic test corpus covering articles, shopping, SPAs, frames, PDFs, lazy loading, authenticated pages, and adversarial content. | CTX-01 |
| QAL-02 | P0 | M | Implement performance telemetry for snapshot latency, context switching, first token, memory, action latency, and stale-response prevention. | CTX-03, MOD-02 |
| QAL-03 | P0 | M | Implement feature flags, kill switches, staged rollout, and fail-to-disabled behavior. Every external provider and action level is independently disableable. | SAF-03 |
| QAL-04 | P1 | L | Build self-host dashboards and release gates for task success, citation correctness, context coverage, crash rate, action failure, and user correction. | QAL-01, QAL-02 |

---

# 11. Critical path

The shortest dependency path to a credible private alpha is:

```text
FND-01/02/03
   ↓
SHL-01 + CTX-01/02
   ↓
CTX-03 + MOD-01/02
   ↓
ANN-01/02 + ACT-01
   ↓
SAF-01/03/04
   ↓
QAL-01/02/03
   ↓
Read-with-me private alpha
```

Do **not** place full clicking or form interaction on the alpha critical path. First prove:

- correct tab;
- correct document;
- correct page context;
- correct selection;
- correct citations;
- correct cancellation;
- correct model switching;
- no cross-workspace leakage.

After those are reliable, implement `ACT-02` through `ACT-07`.

---

# 12. Definition of done for the first useful release

The first release is complete only when:

- The user can select a passage and ask a question inline.
- The response remains anchored or explicitly becomes detached.
- Switching tabs changes AI context automatically.
- Navigating invalidates stale element and document references.
- The assistant can answer from one page or explicitly selected tabs.
- Citations scroll to the supporting passage.
- Returned links can open in a split browsing pane.
- At least two model providers are interchangeable.
- The user can stop an operation end to end.
- Partial page coverage is visible.
- No password, cookie, token, authentication header, or browser-storage content enters snapshots.
- No model response can manipulate a page after its context has become stale.
- Feature flags and kill switches work without requiring a new binary.
- Crash, latency, coverage, cancellation, and correctness dashboards are operating.

The essential product distinction is therefore:

> **The AI is not beside the browser. It participates in the browser’s document identity, selection, navigation, tabs, annotations, results, and actions—while Chromium remains the rendering and compatibility platform.**
