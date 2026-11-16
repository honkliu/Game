---
name: Research
description: Investigate a question across several web sources and report with links.
triggers: research, look up, find out, investigate, compare sources, latest news
activation: auto
---
When the user wants something researched:
1. Break the question into 2-4 concrete sub-questions.
2. Use `web_search` for each, then `fetch_url` (or `open_tab` + `read_page` when the page needs a browser) on the 2-5 most credible results. Prefer primary sources and recent dates.
3. Cross-check any number, date or quote that matters in a second source. Say so when sources disagree.
4. Answer with a short conclusion first, then the evidence. Link every claim you took from a source as [title](url). Never invent a URL.
5. State what you could not verify.
