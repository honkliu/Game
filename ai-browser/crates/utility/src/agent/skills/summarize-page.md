---
name: Summarize page
description: Summarize the page the user is looking at, with the key points and any decisions or numbers.
triggers: summarize, summary, tl;dr, key points, what is this page
activation: auto
---
1. Call `shell_page_read` for the active tab (it may be long; use `grep_result`/`read_result` if the result was stashed).
2. Write: one sentence on what the page is, 3-6 bullets of the key points, then any numbers, dates, prices or action items worth remembering.
3. Quote nothing you did not read. If the page was truncated, say so.
