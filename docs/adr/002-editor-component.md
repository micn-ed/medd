# ADR-002 — CodeMirror 6 for the source pane

**Status:** Accepted, 2026-09-14
**Resolves:** [open-questions.md](../open-questions.md) Q-9 (blocks v0.1)
**Evidence:** [research/q9-editor-component.md](../research/q9-editor-component.md)
**Context:** D-3 (source + preview), D-12 (WYSIWYG is not a goal), E-6, N-1, N-2, N-3

## Decision

The source pane is **CodeMirror 6**, built against directly, with no intervening editor
abstraction layer.

## Why

CodeMirror 6 is the only candidate that satisfies N-1 and N-2 without a fight. Its bundle is two
orders of magnitude smaller than Monaco's — roughly 100–200 KB gzipped against 2–5 MB — which
translates directly into cold-start parse time and resident memory baseline. Both matter more here
than in a typical web app, because this ships inside an application that is expected to stay open
for days rather than in a browser tab with a warm cache.

Its Markdown language support is first-class and actively maintained. `@codemirror/commands` and
`@codemirror/search` deliver undo/redo and find & replace (E-6) as composable extensions rather
than reimplementations, and macOS-correct keybindings assemble from `defaultKeymap` plus targeted
overrides. Its large-document behaviour (viewport-based rendering) is deliberately engineered
rather than incidental, which is what lets the editing side stay live above the size at which the
preview has to degrade (architecture.md §8).

The state/view separation is also what makes the memory design work: inactive tabs retain an
`EditorState` — text plus undo history — without an `EditorView`, bounding memory at O(total text)
rather than O(tabs × editor machinery), while preserving undo history across tab switches.

## Rejected

**Monaco** is the most capable editor here and has the friendlier single-object API, but its weight
is disqualifying for an always-resident app with a sub-second cold-start target. **Ace** is
simpler to integrate than either but is the weakest on large documents and on extension depth.
A **plain textarea with a highlighting overlay** is tempting for its near-zero weight, but E-6's
find & replace and undo/redo would be hand-built, and large-file behaviour would be poor.

## Trade-off accepted

CM6's integration cost is higher than Ace's and arguably higher than Monaco's despite Monaco's
weight. Setting it up means wiring a state field, a view, several extension packages, and a theme
rather than instantiating one object. That is real glue code and a real learning curve —
transactions, state fields, view plugins — and it must be budgeted as such rather than treated as
a drop-in widget. The payoff is a pane that starts fast, stays light, and scales.

## On swappability

D-12 settled that WYSIWYG is not a goal and that the architecture should not pay for editor
optionality. Research independently reached the same place from the other direction: **no seam
makes that swap cheap**, because a rich-text engine's data model is a document tree and a code
editor's is a text buffer, and translating between them *is* the cost. An abstraction layer would
have been a tax with no payout.

What CM6 does buy, for free, is that the *next* increment is cheap: inline Markdown decoration in
the style of Obsidian's live-preview — rejected for v1 in D-3 but the plausible middle ground
before any full WYSIWYG — is a native CM6 extension point (`Decoration`, `ViewPlugin`). Monaco's
decoration API is aimed at diagnostics and diffing, not prose styling; Ace's is weaker still.

This is not licence to let CM6 types leak. Autosave, external-change detection, and tab state talk
to plain text and a derived dirty flag, never to `Transaction` or `EditorState` — ordinary layering,
which happens to also keep the door ajar.

## Consequences

- Real ramp-up time on CM6's extension model must appear in the v0.1 plan as its own line item.
- CM6's Tauri-specific integration is less trodden than Monaco's; WKWebView clipboard and IME
  behaviour must be smoke-tested against the real WebView early, not inferred from browser
  behaviour.
- The bundle-size figures above vary by source and by which packages are counted; they should be
  re-measured against medd's actual bundle rather than trusted as load-bearing numbers.
