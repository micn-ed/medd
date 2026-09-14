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

## Implementation note — avoid the `markdown()` convenience wrapper

Measured during increment 4, and worth recording because the cost is invisible from the import
site. `@codemirror/lang-markdown`'s top-level `markdown()` helper enables embedded-HTML support by
pulling in `@codemirror/lang-html`, which in turn bundles the **complete** `lang-css` and
`lang-javascript` grammars unconditionally — there is no configuration flag that turns this off.
That lands the editor at ~196 kB gzipped, above this ADR's own upper estimate.

medd's source pane has no use for any of it: the preview renders HTML through markdown-it
(ADR-001), never through CodeMirror. Importing the same package's lower-level `markdownLanguage`
and `markdownKeymap` exports directly, rather than the wrapper, keeps GFM-aware parsing and
list-continuation behaviour and drops the three unused grammars entirely — confirmed by inspecting
the built bundle's module graph, not by reading the byte count and assuming.

**Measured: 124.16 kB gzipped**, a 109.5 kB delta over the pre-CodeMirror baseline — inside this
ADR's 100–200 kB estimate, near the low end.

## Consequences

- Real ramp-up time on CM6's extension model must appear in the v0.1 plan as its own line item.
- CM6's Tauri-specific integration is less trodden than Monaco's; WKWebView clipboard and IME
  behaviour must be smoke-tested against the real WebView early, not inferred from browser
  behaviour.
- The bundle-size figures above vary by source and by which packages are counted; they should be
  re-measured against medd's actual bundle rather than trusted as load-bearing numbers.
  **Done — see the implementation note above.** The estimate held, but only after routing around
  a default that would have broken it.
- The macOS keymap arrived as `defaultKeymap` plus **zero** overrides: `@codemirror/commands`
  already carries mac-conditional bindings matching real editor conventions (Cmd-arrows for line
  and document boundaries, Option-arrows for word groups, Cmd/Option-Backspace, Cmd-Z/Cmd-Shift-Z).
  ADR text anticipating "targeted overrides" was pessimistic. `indentWithTab` is the one
  deliberate addition — CodeMirror leaves it off by default because it traps Tab out of focus
  cycling, which is the right default for a code editor embedded in a web page and the wrong one
  for an application whose whole purpose is writing Markdown lists.
