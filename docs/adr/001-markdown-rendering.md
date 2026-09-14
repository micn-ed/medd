# ADR-001 — Render Markdown in the frontend with markdown-it

**Status:** Accepted, 2026-09-14
**Resolves:** [open-questions.md](../open-questions.md) Q-8 (blocks v0.1)
**Evidence:** [research/q8-markdown-rendering.md](../research/q8-markdown-rendering.md)
**Context:** D-2 (Tauri), D-3 (source + live preview), R-1…R-7, N-3

## Decision

Markdown is parsed and rendered to HTML **in the WebView frontend**, using **markdown-it** with
the standard GFM plugin set (tables, task lists, strikethrough, footnotes), fenced code
highlighted by **highlight.js** configured to a curated language subset, and the result sanitised
by **DOMPurify** immediately before insertion into the DOM.

No Markdown parsing happens in Rust.

## Why

The editable buffer necessarily lives in the WebView, because that is where the editor component
runs ([ADR-002](002-editor-component.md)). Given that, rendering in the same process means the
render pipeline never touches the Tauri IPC bridge on the hot path — there is no `invoke` call and
no serialised HTML crossing a boundary on every keystroke. The fastest IPC call is the one that
does not happen, and N-3 is the requirement this most directly serves.

This also collapses sanitisation and rendering into one process's problem rather than a
cross-boundary agreement about what is safe, and it gives medd the deepest available plugin
ecosystem for exactly the deferred features the roadmap already commits to wanting — wikilinks,
math, and Mermaid-as-a-fenced-code-language.

Rust's job in this architecture is file I/O, atomic writes, watching, and workspace state, which
is where a Rust core actually earns its place. Parsing Markdown there would only pay off if
something outside the WebView needed the parsed document — a headless full-text index, say — and
nothing in scope does.

## Rejected

**comrak** (Rust) is the most GFM-faithful option available, being a port of GitHub's own
`cmark-gfm`, and is actively maintained. **pulldown-cmark** (Rust) is faster still, with an event
model well suited to interception. Both were rejected on placement rather than quality: choosing
either means shipping rendered HTML across the IPC bridge on every debounce tick, buying nothing
the frontend cannot do locally.

**remark/unified** (frontend) has the deepest plugin ecosystem of all and a more rigorous
compliance record, but its value proposition is an AST pipeline shared by many downstream
consumers. medd has exactly one consumer of the parsed document — its own preview pane — so
unified's abstraction cost buys nothing markdown-it's simpler plugin model does not already cover.

**shiki** for highlighting is more accurate (TextMate grammars, VS Code quality) but is the
heaviest option, with grammar files per language and ~15 MB unpacked; it is a drop-in-shaped
upgrade later if highlight.js's regex-based approach visibly misbehaves. **Prism** is functionally
stale — last real release March 2025, 488 open issues — and is not a credible choice for new work.
**syntect** (Rust) carries several megabytes of compiled grammars resident permanently, which is
the wrong shape of cost for an app that sits idle all day (N-1).

## Trade-off accepted

markdown-it's CommonMark/GFM compliance is looser at the edges than comrak's — corner cases in
nested emphasis or malformed tables may render slightly differently than github.com would. For a
personal notes and documentation workspace, not a GitHub-preview clone, that gap is very unlikely
to be visible. It buys a materially simpler architecture with no IPC in the render path.

**Escape hatch:** if pixel-perfect GitHub-preview fidelity ever becomes a real requirement, the
replacement is **comrak specifically** — not pulldown-cmark — since comrak's whole reason to exist
is being the closest match to `cmark-gfm`. The golden-file rendering tests (architecture.md §12)
exist partly so that such a swap is a measurable change rather than a leap of faith.

## Consequences

- markdown-it re-parses the whole document per debounce tick. This is what makes the large-document
  thresholds in architecture.md §8 necessary rather than optional.
- The specific GFM plugins chosen (`markdown-it-footnote`, `markdown-it-task-lists`, and
  equivalents) have more variable maintenance quality than the core library; their status should be
  sanity-checked when they are wired up.
- DOMPurify here is correctness, not a security boundary — medd is local-first and offline (N-6),
  so it is defending against the user's own pasted HTML, not an attacker.
