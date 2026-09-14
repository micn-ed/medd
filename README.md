# medd

A resident macOS desktop application for **reading, browsing, and editing Markdown** with a live
side-by-side preview. The feel is Confluence-like: documents are pleasant to read, structured
content renders properly, and the app stays open rather than being launched per task.

Local-first and filesystem-backed. No server, no account, no sync. The workspace is a folder on
disk; the documents are plain `.md` files, and they stay that way.

**Status: design phase.** No implementation code has been written yet. This repository currently
contains the requirements and architecture that the implementation will be built against — that
ordering is a deliberate process requirement, not an accident.

## What it will do

- Open a folder as a workspace, browse it in a collapsible file tree, open documents as tabs
- Split view: real Markdown source on one side, live-rendered preview on the other
- Reading mode: editor hidden, document full-width, typography tuned for actually reading it
- GFM throughout — tables, task lists, footnotes, fenced code with syntax highlighting, images
- Links to other `.md` files in the workspace open as tabs; external links go to the browser
- Debounced autosave with atomic writes, and detection of changes made outside the app
- One instance, reachable from the terminal (`medd notes.md`), from Finder, and from Neovim

## Documentation

Read in this order:

| Document | What it covers |
|---|---|
| [docs/requirements.md](docs/requirements.md) | What the product must do, and for whom |
| [docs/decisions.md](docs/decisions.md) | D-1…D-16 — product decisions, with what was rejected and what each costs |
| [docs/scope-mvp.md](docs/scope-mvp.md) | v0.1 thin slice, then v0.2 and v0.3 |
| [docs/architecture.md](docs/architecture.md) | How it is built |
| [docs/adr/](docs/adr/) | The three technical choices that needed their own argument |
| [docs/research/](docs/research/) | The evidence those three were decided on |
| [docs/plan-v0.1.md](docs/plan-v0.1.md) | The v0.1 build order — twelve increments, ordered by risk |
| [docs/open-questions.md](docs/open-questions.md) | Resolution ledger — all 15 questions, and where each was answered |

## Known limitations

These are deliberate, and worth stating plainly rather than leaving to be discovered.

**The window may not come forward when you open a file from elsewhere.** medd will always open
the file as a tab. Raising and focusing its window is attempted but not guaranteed, because the
macOS API underneath it (`NSRunningApplication.activateWithOptions`) has been unreliable since Big
Sur and is deprecated as of Sonoma — there are open upstream Tauri issues tracing to exactly this,
one of them closed *not planned*. No choice of IPC mechanism fixes it. Rather than build behaviour
that depends on an API that silently declines to work, medd degrades gracefully: your file is
already there when you click the window. See [ADR-003](docs/adr/003-launch-routing.md).

**Very large documents lose the live preview.** Above roughly 1 MB the preview switches to manual
refresh, and above roughly 10 MB the document opens read-only with no preview. Editing stays live
in the first band. The thresholds are tunable constants and get replaced with measured values
during v0.1. See [architecture.md §8](docs/architecture.md).

**macOS only.** Nothing in the design gratuitously prevents Linux later, and the places that are
genuinely macOS-specific — file association, activation, the FSEvents watching strategy — are
called out where they occur so the port is a known quantity rather than a surprise.

## Built with

Rust and [Tauri v2](https://v2.tauri.app/) (macOS WKWebView), with CodeMirror 6 for the source
pane and markdown-it for rendering. The reasoning for each is in the ADRs.

## Licence

[GPL-3.0](LICENSE).
