# Using River

Turn the web into a workspace you can see, arrange, and control through conversation. Canvas puts related pages together. Multiscreen gives that workspace more room. River works with the pages you are using. RiverMCP connects the same live browser to an external AI assistant.

The connection is **context**: the active page, your selection, canvas positions, and your next goal. Show River the relevant material and explain the outcome, without copying every passage into a chat or describing every click.

## Start with one connected workflow

1. **Connect River.** In **Settings > Models**, enter your model name, OpenAI-compatible endpoint, and API key if required, then click **Save model**. Open **River** on the toolbar and choose the model. Browser control requires a model that supports tool calling.
2. **Build a small workspace.** Choose **New canvas** and a two-page layout. Open a product page in one cell and its delivery or returns page in the other.
3. **Give River a job.** Turn **Agent** on and ask: "Read these two pages. Summarize the total cost, delivery time, and return conditions in a comparison table. Read both pages before answering."
4. **Keep working together.** Select an unclear condition and ask what it means. Ask River to open another source in a particular cell, or take over and browse there yourself. Check the context display when changing the page you want to discuss.

## Canvas: work with pages together

A canvas is a grid of **live, interactive webpages**, not a collection of screenshots. Keep sources, comparisons, and the website you are working on visible in one place. Each cell remains a page you can read and interact with.

### Create and shape a workspace

Choose **New canvas**, then select a grid. Click a cell and use the address bar to open its page. Use **Canvas layout** to change the arrangement as the task grows.

Drag the dividers to give a detailed page more room; double-click a divider to restore equal proportions. **Zoom canvas page** enlarges the focused page, then returns you to the grid when used again. The canvas close-page control closes the focused page; **Close canvas** is a separate action in the canvas menu.

### Focus is how you point

Click the page you want to work with. That cell becomes the **current page** for the address bar and River. "Explain this page" follows the focused cell, not the canvas as a whole.

For work across cells, name the sources or their positions:

> "Read the specifications on the left and the warranty on the right. Tell me which advertised features have warranty exclusions."

River receives the canvas membership and cell positions as part of browser context. Ask it to read the relevant pages explicitly; having several pages visible does not mean every page's full contents have already been read.

### Arrange pages through conversation

You can ask River to organize a canvas instead of doing all the setup yourself:

> "Create a two-column canvas. Open the documentation link from this page on the left and its getting-started example on the right."

Use positions such as "left", "right", or "top-right" and describe the page you mean. For an existing workspace, say whether you want a page replaced or a new canvas created.

## Multiscreen: give the workspace more room

On Windows, **Span monitors** expands one browser window across selected displays. Combined with Canvas, this lets you spread related pages over a larger working area while keeping one River conversation and one browser context.

### Choose the displays

1. Connect the displays you want to use.
2. Open **Choose monitors**, beside the **Span monitors** button in the window controls.
3. Select **2 screens side by side**, **2 screens stacked**, or **4 screens (2 by 2)**.
4. Assign a different connected monitor to each position, then click **Save and span**.

The monitor choices include their numbers, resolution, and scale. Match the positions to your physical arrangement. River saves your selection without changing Windows display settings.

Use **Span monitors** again to restore the previous window. If the button is unavailable, check that a second distinct display is connected. Reopen **Choose monitors** if a saved display has been disconnected.

### Pair the screen layout with the canvas

Spanning makes the window larger; Canvas determines how pages divide that space. After spanning, choose a canvas layout and adjust its dividers so the sources you need remain comfortably visible.

You can also describe the combined setup to River:

> "Use my saved monitor selection to span the window, then arrange these pages in a two-by-two canvas."

Focus still follows the page you click, regardless of which monitor contains it.

## River: understand the page or act on it

River is connected to the browsing workspace, rather than being a separate chat that needs everything pasted into it.

| What you want | Choose | How to use it |
| --- | --- | --- |
| Understand or question page content | **Agent off** | Ask about the active page, use selected text, and follow supporting citations. |
| Navigate, interact, or complete a browsing task | **Agent on** | Describe the outcome; River can read pages and use browser tools. |

Change models with the **Model** button. Tool-calling support is configured under **Advanced** in **Settings > Models**.

### Describe the result, not a script of clicks

A useful request identifies the material, the work, and the stopping point:

> "Use the delivery information in this canvas to fill the shipping options on the current form. Leave any missing information blank and stop before submitting."

River can read pages, open links, click controls, fill fields, and use connected tools. Website requirements still apply: complete a login or other required manual step yourself, then tell River where to continue.

During a run, follow the tool activity in the conversation. Model-provided thinking appears separately from the answer. Click **Stop** to interrupt; it does not reverse actions already completed.

## Context: show River what you mean

### The active page and open workspace

River receives the active page's identity, selected text when present, and a list of open pages with canvas locations. It can read page content through browser tools as needed.

When several pages have similar titles, identify the site, document, or canvas position. An open-tab list is not the full contents of every tab.

### Pass a passage through selection

Select text on a webpage to bring it into River's context. Use **Explain**, **Translate**, **Verify**, or **Rewrite**, or write your own request about the selection.

> "Compare this selected exception with the policy in the right-hand cell. Are they describing the same restriction?"

Check the selection card to see exactly which passage your question refers to.

### Include more sources in a page question

With **Agent off**, use **Also read** to select additional available tabs for a question. This is useful when you want a cited explanation spanning several sources without a browser-control workflow.

With **Agent on**, tell River which pages to read and compare. It can use the workspace's tab and cell information to find them. Be explicit when your conclusion depends on more than the active page.

### Inspect or refine the context

Open **View / edit context** to inspect what River receives about the browser. Use it to check the active source, remove an irrelevant detail, or add task-specific background.

Edits affect **Agent mode**; page-question mode uses the passages it cites. When the page changes, the edit is discarded and context follows the page again. Use **Settings > Instructions** for a preference that should apply to every Agent run, rather than adding it repeatedly to page context.

If information is missing, expand the relevant section or let it finish loading, then ask River to read it again. Content in inaccessible frames or media without transcripts may not be available.

## Combine navigation and conversation

**Auto / Manual** controls how River handles navigation in Agent mode. In **Auto**, River can open pages itself. In **Manual**, it offers links for you to open. Manual is useful when you want to choose the next source while River helps interpret it.

You can alternate between acting yourself and asking River to continue. After navigating or changing a form, state the next goal rather than repeating the entire task:

> "I have selected the delivery option. Review the current page against the requirements we discussed, and list anything still missing."

For a new, unrelated task, click **New**. Previous conversations are available in **Settings > Saved chats**. Keep the same conversation when a follow-up depends on the earlier comparison or instructions.

## RiverMCP: connect an external assistant to this workspace

River is the assistant inside the browser. **RiverMCP** lets an MCP-capable external client, such as a coding assistant, work with the same live browsing session. It can request browser context, read pages, and use browser controls instead of relying only on a pasted URL or screenshot.

### Connect a client

Open **Settings > RiverMCP**. Add an HTTP MCP connection in your external client using the displayed **local endpoint** and **token**. Use that client's authentication settings for the token; it is not your model API key.

For a client on another computer, replace the local address with this computer's IP address and keep the active port shown in Settings. If you change the port or token, click **Apply** and update the client too. Keep River running while the client is using it.

### Pass the task across applications

Tell the external assistant what browser material to use and what to do with it:

> "Read the API documentation open in River, including the selected example. Use it to implement this request in my project, then open the local preview in the browser."

This connects browser research to work in another application. Be clear about the context to read; do not assume the client has already loaded every page or River's conversation history. Actions from the client affect the live browser workspace.

**MCP and RiverMCP work in opposite directions.** **Settings > MCP** adds external tools for River to use. **Settings > RiverMCP** gives another assistant access to this browser.

## Controls that shape a workflow

Tools are enabled by default through one switch in **Settings > Tools**. In **Settings > General**, under **River**, **Run actions without asking** controls automatic action approval, and **Max tool rounds per run** sets the saved limit for a task. Click **Save** after changing these settings.

Tool availability, navigation mode, and approval are separate choices: a tool can be available while an action still waits for approval. For repeated workflows, **Settings > Skills** can hold a reusable playbook, such as a comparison checklist, so your requests can focus on the material at hand.

## Platform notes

| Feature | Windows | macOS |
| --- | --- | --- |
| Canvas and River | Use the workspace and context workflows above. | Use the same workflows. |
| Multiscreen window spanning | Use **Choose monitors** and **Span monitors**. | The built-in **Span monitors** control is not available. |

Find the installed app version in **Settings > About**.
