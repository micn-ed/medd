# medd

A resident macOS desktop application for **reading, browsing, and editing Markdown** with a live
side-by-side preview. The feel is Confluence-like: documents are pleasant to read, structured
content renders properly, and the app stays open rather than being launched per task.

Local-first and filesystem-backed. No server, no account, no sync. The workspace is a folder on
disk; the documents are plain `.md` files, and they stay that way.

**Status: implementation, v0.1 in progress.** The skeleton (increment 1 of
[docs/plan-v0.1.md](docs/plan-v0.1.md)) is in place: a Tauri v2 window, a Svelte frontend, and one
command proving the IPC bridge works. Nothing else works yet — no file reading, no editor, no
preview.

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
the file as a tab; raising and focusing its window is attempted, and in one case cannot be
guaranteed. If the window is merely behind another app, the macOS API underneath
(`NSRunningApplication.activateWithOptions`) has been unreliable since Big Sur and is deprecated as
of Sonoma — there are open upstream Tauri issues tracing to exactly this, one closed *not planned*,
and no choice of IPC mechanism fixes it. If the window is minimised (Cmd+M) or hidden (Cmd+H) —
both of which are standard items in medd's own menu — medd restores it first, which is what makes
activation work in those states rather than silently doing nothing. Where it still fails, medd
degrades gracefully: your file is already there when you click the window.
See [ADR-003](docs/adr/003-launch-routing.md).

**Very large documents lose the live preview.** Above roughly 1 MB the preview switches to manual
refresh, and above roughly 10 MB the document opens read-only with no preview. Editing stays live
in the first band. The thresholds are tunable constants and get replaced with measured values
during v0.1. See [architecture.md §8](docs/architecture.md).

**Two launches at the exact same instant can produce two windows.** medd is meant to be a single
process, and in ordinary use it is. The single-instance mechanism it relies on removes and
recreates its lock in two steps rather than one, so two launches landing inside that window can
both believe they are the first. Rare, and not fixable without replacing the mechanism; recorded
rather than hoped over. See [ADR-003](docs/adr/003-launch-routing.md).

**Images hosted on the web will not render.** A `![](https://…)` in a document shows a
broken-image icon. This is deliberate: medd works fully offline (N-6) and holds a content
security policy that permits no network origin at all, which is what stops a document being able
to carry anything off the machine by requesting a remote resource. Images stored beside the
document render normally, as do base64 data URIs — the restriction is specifically on fetching
from the network.

**macOS only.** Nothing in the design gratuitously prevents Linux later, and the places that are
genuinely macOS-specific — file association, activation, the FSEvents watching strategy — are
called out where they occur so the port is a known quantity rather than a surprise.

## Building

Requires [rustup](https://rustup.rs) and Node. The Rust toolchain is pinned in
[`rust-toolchain.toml`](rust-toolchain.toml) so every build uses the same compiler — a
Homebrew- or system-installed `cargo` ignores that file, so install Rust through rustup rather
than a package manager. Node is only ever a build-time dependency; nothing ships it.

```sh
npm install
make dev      # run in development
make build    # produce the .app bundle
```

## Built with

Rust and [Tauri v2](https://v2.tauri.app/) (macOS WKWebView), with CodeMirror 6 for the source
pane and markdown-it for rendering. The reasoning for each is in the ADRs.

## Licence

[GPL-3.0](LICENSE).
