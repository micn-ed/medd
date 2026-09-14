# Q-9 — Which code editor component?

**Status:** Architecture research, September 2026
**Resolves:** [open-questions.md](../open-questions.md) Q-9 (Blocks v0.1)

---

## The question

The source pane needs real editing, not a glorified `<textarea>`: Markdown syntax highlighting,
undo/redo, find & replace (E-6), and macOS keybinding fidelity, all inside a Tauri WebView that
must reach a usable window in under a second (N-2) and hold a modest, stable memory footprint
across multi-day sessions (N-1). Q-6 adds a second axis: if WYSIWYG turns out to matter to the
user beyond v1, the editor slot should not be welded shut against a rich-text replacement.

## Options

**CodeMirror 6.** The current release line is at `@codemirror/state` 6.7.4 (published within
days of this research, per npm), with the wider `@codemirror/*` family on parallel 6.x versions
and `@codemirror/lang-markdown` actively maintained as a separate, composable package. CM6 is a
full rewrite from CM5, built as roughly forty small npm packages around a core view/state
architecture, and it is the editor Replit rebuilt its product on (their engineering blog is
explicit about why: bundle size and the ability to reason about editor state as an immutable,
transactional data structure). Reported gzip sizes for a realistic CM6 setup — core, view, one
language mode, search, basic keymaps — land around 100–200KB, and the architecture only pays for
what you import: no LSP client, no minimap, no unused language modes bundled in by default.
Markdown support comes from `@codemirror/lang-markdown`, which extends CommonMark with GFM,
plus configurable embedded-language highlighting inside fenced code blocks — exactly the shape
of the R-3/R-5 requirements. CM6 explicitly engineers for huge documents: its own "million line"
demo document loads and scrolls smoothly, achieved by viewport-windowed rendering and by
disabling expensive per-line work (full syntax highlighting, bracket matching) past a line-length
threshold rather than doing it unconditionally. It is not immune to pathological cases — long
synchronous operations like full-document diffing/merge on very large files have been reported
as slow on the CodeMirror forums — but plain typing-and-highlighting performance on documents in
medd's realistic size range (project Markdown, not multi-hundred-thousand-line logs) is a solved
problem for this engine. Its downside is API surface: CM6's extension system is powerful but has
a real learning curve, and undo/redo, search, and keymaps are separate packages you wire together
rather than a single `new Editor()` call.

**Monaco Editor.** The VS Code editor, extracted for the web; latest npm release 0.56.0 as of two
months before this research, with thousands of dependents and no sign of the project slowing
down. Its editing quality is excellent and it's the most familiar of the three to anyone who
uses VS Code daily. But it was built for a browser tab backed by a multi-process browser engine
and, in VS Code's real deployment, a Node/Electron host doing the heavy lifting — not for
embedding cold inside a single WebView on every launch. Bundle-size comparisons vary by exactly
what's counted, but the consistent finding across sources (Sourcegraph's own migration post-
mortem, community bundle-size trackers) is that Monaco lands somewhere between roughly 2MB and
5MB gzipped depending on which language/worker bundles are included, against CM6's 100–200KB for
an equivalent feature set — over an order of magnitude larger. For a Tauri app, that bundle ships
inside the `.app` and is parsed and evaluated locally on every cold start, directly working
against the sub-1-second N-2 target; Sourcegraph's writeup and Replit's both cite exactly this
kind of load-time and integration-friction concern as why they moved away from Monaco toward
CM6. Monaco also runs its language services (tokenizing, in Sourcegraph's more advanced case LSP)
on background workers, which is the right call for a full IDE but is unused weight for a
Markdown-only editing surface, and those workers are a second thing that must warm up before the
window is "usable." None of this makes Monaco a bad editor — it makes it a poor fit for an
always-resident, cold-launch-sensitive, single-language editing pane.

**Ace Editor.** Latest `ace-builds` on npm is 1.44.0 (~4 months old at research time), with
Wikipedia recording 1.43.6 as a more recent "stable" tag from March 2026 — a small discrepancy
that itself signals a slower, more conservative release cadence than CM6's. Ace is still actively
maintained (it's Cloud9's editor, historically backed by Mozilla-adjacent tooling), and it is
lighter than Monaco and simpler to integrate than CM6 — closer to a drop-in `<div>` + `require`.
But it is the oldest architecture of the three, its Markdown mode is functional rather than
excellent, and it has neither CM6's modern extension ecosystem nor Monaco's feature depth. It
would be a reasonable choice if the goal were "cheapest possible integration," but on every axis
that matters here — bundle size vs. Monaco, editing/extension quality vs. either — it's dominated
by one of the other two rather than being the best answer on its own terms.

**Plain `<textarea>` + overlay highlighting.** A `<textarea>` for input with a positioned,
transparent overlay `<div>` re-rendering highlighted HTML behind it (the classic
"contenteditable-avoidance" trick used by tools like the react-simple-code-editor family) is the
smallest possible bundle and the simplest mental model. It's disqualified for this project on
requirements grounds, not performance grounds: E-6 asks for real find & replace and clean
undo/redo, and a bare textarea's native undo stack is coarse (browsers coalesce keystrokes
unpredictably and offer no scriptable access to it), Cmd+F within the pane has to be entirely
hand-rolled with manual selection/scroll manipulation, and macOS keybinding fidelity (word-jump,
Option+arrow, etc.) is whatever the browser's native textarea happens to do — no way to extend
or correct it. This option is worth naming because it's the honest zero-dependency baseline, but
it would mean rebuilding, badly, features CM6 already ships correctly.

## Recommendation

CodeMirror 6. It is the only option that satisfies N-1 and N-2 without a fight: a two-orders-of-
magnitude smaller bundle than Monaco means faster parse/eval on cold start and a lower resident
memory baseline, both of which matter more here than in a typical web app because medd is
designed to stay open for days. Its Markdown language support is first-class and actively
maintained as of this research (days-old releases on the core packages), its large-document story
is deliberately engineered rather than incidental, and undo/redo, search/replace, and custom
keymaps are all real, composable extensions rather than reimplementations — `@codemirror/commands`
and `@codemirror/search` give E-6 essentially for free, and macOS-correct keybindings can be
assembled from CM6's `defaultKeymap` plus targeted overrides rather than built from scratch.

## Trade-off accepted

CM6's integration cost is higher than Ace's and arguably higher than Monaco's despite Monaco's
weight — you are wiring together a state field, a view, a handful of extension packages, and a
theme, rather than instantiating one object. That is real, non-trivial glue code, and it is the
price of only paying for what you use. The team should budget real time for learning CM6's
extension model (transactions, state fields, view plugins) rather than treating it as a drop-in
widget; the payoff is a pane that starts fast, stays light, and scales to the document sizes the
product actually expects.

## Swappability (Q-6)

The way to keep the WYSIWYG door open without paying for it now is to treat the source pane as a
component behind a narrow, editor-agnostic interface — something like "given a path, mount an
editable view of this text, emit change events with the transformed content, expose undo/redo and
find/replace commands" — rather than letting CM6-specific types (`Transaction`, `EditorState`,
`ViewPlugin`) leak into the rest of the app (tab management, autosave debouncing, the preview
sync logic). Concretely: the autosave and external-change-detection logic (P-1–P-4) should talk
to "current buffer text" and "buffer dirty since disk write," not to CM6's internals; the
tab/pane layout should mount "the source view for this tab" as an opaque unit. Under that
discipline, replacing CM6 with a rich-text engine later is a full component rewrite regardless of
today's choice — a WYSIWYG editor's data model (a document tree, not a text buffer) is
fundamentally different from a plain-text editor's, so there is no seam shallow enough to make
that swap cheap no matter which of these four options is picked today. What CM6 buys over the
alternatives is that the *interim* steps toward richer editing — inline Markdown decoration in
the style of Obsidian's live-preview mode, which was explicitly rejected for v1 in D-3 but is a
plausible v2 middle ground before a full WYSIWYG rewrite — are native CM6 extension points
(`Decoration`, `ViewPlugin`) rather than a separate engine. Monaco offers no comparable middle
ground (its decoration API is aimed at diagnostics/diffing, not prose styling), and Ace's is
weaker still. So while nothing here makes a future WYSIWYG layer cheap, CM6 keeps the *next*
increment cheap, which is the more likely path anyway given D-3's framing of hybrid inline-styled
source as "not ruled out forever."

## Risks / unknowns

CM6's extension ecosystem, while extensive, is still thinner for Tauri-specific integration
(clipboard/IPC quirks inside a WebView vs. a real browser tab) than either Monaco's or plain
textareas' — this should be smoke-tested early against the actual Tauri WebView, not assumed from
browser behavior, since WKWebView on macOS has its own history of clipboard and IME edge cases.
The "disable highlighting past a line-length threshold" behavior is a real mitigation for
pathological single-line files (e.g., minified JSON pasted into a fenced block) but its exact
threshold and visual effect should be verified against whatever "large document" ends up meaning
for Q-4, once that's answered. Finally, the specific gzip-size figures cited above vary by source
and by exactly which packages are counted as "the app's editor" — they should be re-measured
against medd's actual bundle once the dependency set is locked, rather than trusted as load-bearing
numbers.

## Sources

- [@codemirror/state — npm](https://www.npmjs.com/package/@codemirror/state) — 6.7.4, published days before research (Sept 2026)
- [@codemirror/lang-markdown — npm](https://www.npmjs.com/package/@codemirror/lang-markdown)
- [codemirror/lang-markdown — GitHub](https://github.com/codemirror/lang-markdown)
- [CodeMirror Huge Doc Demo](https://codemirror.net/examples/million/)
- [CM6 Performance Benchmarks — discuss.CodeMirror](https://discuss.codemirror.net/t/cm6-performance-benchmarks/2471)
- [Tips for improving codemirror performance — discuss.CodeMirror](https://discuss.codemirror.net/t/tips-for-improving-codemirror-performance/1331)
- [CodeMirror Merge Slow Diff — discuss.CodeMirror](https://discuss.codemirror.net/t/codemirror-merge-slow-diff/7005) (large-file diff pathology)
- [Betting on CodeMirror — Replit Engineering](https://blog.replit.com/codemirror)
- [Migrating from Monaco Editor to CodeMirror — Sourcegraph](https://sourcegraph.com/blog/migrating-monaco-codemirror)
- [monaco-editor — npm](https://www.npmjs.com/package/monaco-editor) — 0.56.0, published ~2 months before research
- [ace-builds — npm](https://www.npmjs.com/package/ace-builds) — 1.44.0, published ~4 months before research
- [ajaxorg/ace — GitHub](https://github.com/ajaxorg/ace)
