# medd — Open Questions

**Status: all resolved, 2026-09-14.** Every question below has an answer recorded in
[decisions.md](decisions.md), [architecture.md](architecture.md), or an
[ADR](adr/). Nothing blocks v0.1 implementation.

The original text of each question is preserved for the record, with its resolution noted
directly beneath it. The "for the user" / "for the architecture phase" split reflects how the
questions were triaged during requirements gathering, not who ultimately answered them — Q-1,
Q-2, Q-6 and Q-7 were put to the user directly; Q-3, Q-4 and Q-5 were decided by the project
lead as tactical calls; Q-8, Q-9 and Q-10 were researched before being decided.

## Resolution index

| # | Question | Resolved in |
|---|---|---|
| Q-1 | External change to an open file | D-11 |
| Q-2 | Changing workspace; bare-launch empty state | D-14 |
| Q-3 | Opening a file from outside the workspace | D-15 |
| Q-4 | Large-document degradation | architecture.md §8 |
| Q-5 | Outline / table of contents priority | D-16 |
| Q-6 | Does WYSIWYG still matter | D-12 |
| Q-7 | Repository visibility and licence | D-13 |
| Q-8 | Markdown parser and renderer | ADR-001 |
| Q-9 | Code editor component | ADR-002 |
| Q-10 | Reaching the running instance | ADR-003 |
| Q-11 | Defining "no unbounded memory growth" | architecture.md §8 |
| Q-12 | Where application state lives | architecture.md §7 |
| Q-13 | Filesystem watching strategy | architecture.md §6 |
| Q-14 | CLI distribution | architecture.md §9 |
| Q-15 | Testing strategy | architecture.md §12 |

---

## For the user

### Q-1 — What happens when a file changes on disk while it is open? **Blocks v0.1**

**Resolved** → [decisions.md](decisions.md) D-11 — silent reload when clean, banner when dirty.

Autosave (D-5) makes this unavoidable: the user edits in Neovim, or checks out a branch, and the
open buffer is now stale. Silently overwriting would destroy work.

Options: (a) if the in-app buffer is unmodified, reload silently; (b) always show a
non-blocking "changed on disk — reload?" banner; (c) attempt a merge.

Recommendation: (a) plus (b) — silent reload when there is nothing to lose, an explicit prompt
when there is. (c) is out of proportion for v1.

### Q-2 — How does the user change workspace, and what opens on a bare launch?

**Resolved** → [decisions.md](decisions.md) D-14 — Open Folder… + recent workspaces, welcome empty state.

`medd` with no arguments should restore the last workspace (v0.2), but the first-ever launch has
nothing to restore, and there is no defined way to switch root folders once running.

Needs: a "Open Folder…" action, a recent-workspaces list, and a defined empty state.

### Q-3 — What happens when a single `.md` file is opened from outside any workspace?

**Resolved** → [decisions.md](decisions.md) D-15 — loose tab, tree unchanged.

Opening `~/Downloads/readme.md` from Finder — does its parent directory become the workspace,
does it open as a loose tab in the current workspace, or does the tree show its siblings?

Recommendation: open as a tab in the current workspace, with the tree unchanged. Cleanest
mental model, but worth confirming.

### Q-4 — Should very large documents degrade gracefully, and at what size?

**Resolved** → [architecture.md](architecture.md) §8 — live under 1 MB, manual-refresh preview to 10 MB, read-only source above.

Live preview on every keystroke has a ceiling. Is there a realistic document size (a 5MB log
dump, a generated API reference) that must still open, even if the preview updates lazily?

### Q-5 — Is a document outline / table of contents wanted sooner than "later"?

**Resolved** → [decisions.md](decisions.md) D-16 — stays deferred; strongest candidate to promote after v0.3.

Listed as a later candidate, but for Confluence-like *reading* of long documents it is arguably
more valuable than several v0.3 items. Worth a explicit yes/no on its priority.

### Q-6 — How much does WYSIWYG still matter?

**Resolved** → [decisions.md](decisions.md) D-12 — not a goal; no editor abstraction layer is paid for.

Rejected for v1 (D-3), but the original brief said "Confluence-like editing experience", and
Confluence is WYSIWYG. If this is a genuine long-term goal rather than a nice-to-have, the
architecture phase should keep the editor layer swappable — which is a real constraint worth
knowing about now.

### Q-7 — Repository visibility and licence?

**Resolved** → [decisions.md](decisions.md) D-13 — public on GitHub, GPL-3.0.

The project will live on GitHub. Public or private? If public, which licence?

---

## For the architecture phase

### Q-8 — Which Markdown parser and renderer? **Blocks v0.1**

**Resolved** → [adr/001-markdown-rendering.md](adr/001-markdown-rendering.md) — markdown-it + highlight.js + DOMPurify, in the frontend.

Affects GFM coverage, extension points (wikilinks later), incremental re-render, and whether
rendering happens in Rust or in the frontend. Also determines the sanitisation story for HTML
embedded in Markdown.

### Q-9 — Which code editor component? **Blocks v0.1**

**Resolved** → [adr/002-editor-component.md](adr/002-editor-component.md) — CodeMirror 6, no abstraction layer.

The source pane needs Markdown syntax highlighting, undo/redo, find & replace, and good
behaviour on large files. This is the single biggest frontend dependency, and Q-6 makes its
swappability a live concern.

### Q-10 — How does a second launch reach the running instance? **Blocks v0.1**

**Resolved** → [adr/003-launch-routing.md](adr/003-launch-routing.md) — two listeners, one router; activation best-effort.

Single instance (D-7) requires an IPC channel — the CLI, Finder, and the Neovim plugin all need
to hand a path to a running process and raise its window. One mechanism should serve all three;
choosing it is an architecture decision, but it is on the critical path for v0.1's CLI.

### Q-11 — What does "no unbounded memory growth" mean in practice?

**Resolved** → [architecture.md](architecture.md) §8 — <150 MB target, +10% over 8h criterion, active-tab-only views.

N-1 sets a goal without a number. The architecture phase should define an idle-memory target
and a tab-count behaviour (are closed tabs' buffers freed? is there a cap on open tabs?) so the
requirement is testable rather than aspirational.

### Q-12 — Where does application state live on disk?

**Resolved** → [architecture.md](architecture.md) §7 — `~/Library/Application Support/medd/`, settings.json + state.json, atomic, corruption non-fatal.

Preferences, recent workspaces, session state. Needs a defined location and format, and a
decision on whether it is human-editable.

### Q-13 — Filesystem watching strategy

**Resolved** → [architecture.md](architecture.md) §6 — recursive FSEvents watch on the root plus loose-file parents, coalesced.

Required by Q-1 (external change detection) and by live tree updates. Watching a large workspace
recursively has real cost; the scope of watching needs deciding.

### Q-14 — How is the CLI distributed?

**Resolved** → [architecture.md](architecture.md) §9 — shell shim, warm path via the plugin socket, cold path via `open -a`.

`medd` must be on `PATH`, but the app itself is a `.app` bundle. Symlink on first run, a shipped
shell shim, a Homebrew formula? Affects v0.1 since the CLI is the only v0.1 launch path.

### Q-15 — Testing strategy

**Resolved** → [architecture.md](architecture.md) §12 — data-loss surface tested automatically, taste and platform quirks by hand.

Not discussed at all. What is tested automatically — Markdown rendering fidelity, autosave
atomicity, IPC routing — and what is verified by hand? Worth settling before code exists rather
than after.
