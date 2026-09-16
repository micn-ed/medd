# medd — Decisions

Product-level decisions taken during requirements gathering, with rationale and the
alternatives that were rejected. These are **requirements-stage decisions**; technical
architecture decisions (crate choices, IPC mechanism, render pipeline, state model) belong to
the architecture phase and are not recorded here.

Format: each decision states what was chosen, why, what was rejected, and what it costs.

---

## D-1 — Implementation language: **Rust**

**Decision.** The application is written in Rust.

**Rationale.** The app is resident — open all day, every day — so idle footprint and the
absence of GC pause behaviour matter more than they would for a tool launched per task. Rust
also has, by a wide margin, the better desktop GUI ecosystem for this shape of application.

**Rejected: Go.** Faster to write and an excellent CLI and daemon story, but its desktop GUI
options are genuinely weak — Fyne is limited, and Wails puts you in a webview anyway, at which
point Rust's equivalent is strictly better. Go's GC also gives a higher idle memory floor for a
process expected to sit idle most of the time.

**Cost.** Steeper development ramp; slower initial velocity than Go.

---

## D-2 — GUI stack: **Tauri (Rust core + web frontend)**

**Decision.** Rust backend, web-technology frontend rendered in the macOS system WebView, via
Tauri.

**Rationale.** The requirements are dominated by *rendering rich documents beautifully* —
tables, syntax-highlighted code, images, typography that reads like a published page. That is
precisely what a browser engine is best in the world at, and rebuilding it is a large,
low-value effort. Using the system WebView rather than bundling a browser keeps the binary
small and the memory cost far below an Electron-class app.

**Rejected: egui / iced (pure Rust rendering).** Lighter still and fully native, but every
piece of Markdown presentation — table layout, text wrapping, code highlighting, image
placement, selection behaviour — would be hand-built. Enormous effort spent recreating a layout
engine, for a worse result.

**Rejected: TUI (ratatui).** Pairs beautifully with the Neovim and CLI launch paths, but has no
credible "open from Finder" story and cannot deliver the Confluence-like reading experience
that is the point of the product.

**Rejected: native macOS (Swift bridge / GPUI).** The most native possible feel, at the cost of
hard macOS lock-in and substantially more complexity for benefit the user did not ask for.

**Cost.** A web frontend inside a Rust project means two languages and a build step. The
WebView is the platform's, so rendering behaviour is tied to the OS version.

---

## D-3 — Editing model: **source + live preview, plus a reading mode**

**Decision.** The editor pane always shows real Markdown source. A preview pane renders it live
alongside. A separate **reading mode** hides the editor and presents the rendered document
full-width for consumption rather than authoring.

**Rationale.** This was the user's explicit request, and it keeps the product honest: what is on
screen in the editor is exactly what is in the file, so there is never a question of what a
save will produce. The reading mode addresses the other half of the use case — these documents
are read far more often than they are written, and a half-width column beside a text editor is
a poor reading experience.

**Rejected: true WYSIWYG.** Closest to Confluence, but requires a rich-text editing engine plus
lossless Markdown round-tripping — a large project in itself, and a well-known source of subtle
file corruption. Not ruled out forever; see [open-questions.md](open-questions.md).

**Rejected: hybrid inline-styled source (Obsidian-style live preview).** A reasonable middle
ground, but it dilutes both modes rather than doing either well, and the user asked for the
split.

**Cost.** Less "magical" than Confluence. The user sees Markdown syntax while writing.

---

## D-4 — Workspace model: **folder workspace, tabs, collapsible tree**

**Decision.** The app opens a root folder. A sidebar file tree browses it; documents open as
tabs; the sidebar can be collapsed away entirely.

**Rationale.** A resident app needs somewhere to reside. A folder workspace gives the file tree
and the quick-open dialog something to operate on, and tabs let the user keep several related
documents in play — which is how documentation is actually read. Collapsing the tree is what
makes reading mode genuinely full-width.

**Rejected: single-file-at-a-time.** Simpler, but leaves the app with nothing to be resident
*about*, and makes the file tree requirement awkward.

**Rejected: multi-root workspaces.** VS Code-style multiple roots is real power, but it is
scope the user did not ask for and complicates the tree, quick-open, and link resolution.

**Cost.** Needs a concept of "current workspace" and a way to switch it.

---

## D-5 — Save behaviour: **debounced autosave**

**Decision.** Buffers are written to disk automatically, shortly after typing stops. There is no
save prompt and no unsaved-changes state to manage.

**Rationale.** This is the Confluence-like behaviour the product is aiming for, and it removes
an entire class of user anxiety. It also makes the Neovim story cleaner — the file on disk is
always current, so switching tools never loses work.

**Rejected: manual Cmd+S only.** Familiar, but reintroduces dirty state, close prompts, and lost
work on crash.

**Rejected: manual + crash-recovery journal.** More machinery than autosave for a strictly worse
user experience here.

**Cost.** Autosave makes two things load-bearing that would otherwise be minor:
**atomic writes** (a crash mid-write must not truncate the user's document) and **external
change detection** (the file may also be open in Neovim or being rewritten by git). Both are
promoted to Must in the requirements as a direct consequence of this decision.

---

## D-6 — Neovim integration: **one-way hand-off**

**Decision.** Neovim gets a command that sends the current buffer's file path to the running
medd instance, which opens it in a tab and takes focus. Nothing more.

**Rationale.** This satisfies the stated requirement with a small, robust mechanism that uses
the same entry point as the CLI. It has almost no ongoing maintenance surface.

**Rejected: live preview of the unsaved nvim buffer.** Genuinely attractive — keep editing in
Neovim, watch medd render it — but it requires a persistent RPC channel and buffer-change
streaming, and turns medd into a viewer for another editor rather than an editor in its own
right. Deferred, not dismissed.

**Rejected: two-way sync.** Two live editors on one file means conflict resolution. Not worth it.

**Cost.** Hand-off only. Since autosave (D-5) keeps the file current, the practical gap is
smaller than it sounds: save in Neovim and medd's preview can follow via file watching.

---

## D-7 — Process model: **single instance, single window**

**Decision.** One process. A second launch — from Finder, the CLI, or Neovim — passes its
arguments to the running instance and exits immediately. Closing the window quits the app.

**Rationale.** The three launch paths (Finder, CLI, Neovim) only make sense if they converge;
three processes each holding a WebView would defeat the memory goal entirely. Single instance
also gives one coherent place for workspace state.

**Rejected: background daemon with a detachable window.** Instant reopen and persistent state,
but it is a second process to supervise, a lifecycle to explain, and a support burden — for a
benefit that matters only if the user closes the window often, which by definition they do not.

**Rejected: multiple windows in one process.** Useful for multiple workspaces; unnecessary while
the workspace model is single-root (D-4).

**Cost.** No window-per-workspace. Quitting loses in-memory state unless it has been persisted.

**Amendment (2026-09-16): "one process" cannot be fully guaranteed, and this decision should say
so on its face.** The single-instance mechanism medd relies on unlinks its lock path and *then*
binds, so two launches landing inside that window both unlink and both bind — two primaries, two
windows, with the first listener orphaned on an unlinked inode and permanently unreachable. I-1 is
a **Must** and this is the one case it does not hold.

Vanishingly unlikely in single-user desktop use, and not fixable at medd's layer without replacing
the mechanism. It is recorded in the README's known limitations and argued in
[adr/003-launch-routing.md](adr/003-launch-routing.md) — but **this is where someone reasoning about
the process model will look**, and a locked decision whose stated property is known unachievable
should carry the exception rather than leave it to be discovered two documents away. The decision
itself is unchanged; only its claim is now honest.

---

## D-8 — Image support: **basic rendering only**

**Decision.** Images referenced in Markdown render in the preview, with relative paths resolved
against the document's own directory. No clipboard paste-to-insert, no drag-and-drop, no
resizing.

**Rationale.** The user did not select images among the features that matter, and confirmed on
follow-up that rendering alone is sufficient. Rendering is nearly free given D-2; authoring
workflows are where the real cost lives, and there is no demand for them.

**Cost.** Adding images to a document means writing the Markdown by hand.

---

## D-9 — Search: **quick-open and in-file find only**

**Decision.** v1 ships fuzzy quick-open by filename (Cmd+P) and find/replace within the current
document (Cmd+F). Full-text search across the workspace is deferred.

**Rationale.** Quick-open *is* the "find and click a file" requirement from the original brief,
so it is not optional. In-file find is table stakes for an editor. Workspace-wide full-text
search needs an index, an incremental update strategy, and a results UI — a feature in its own
right, and the user did not ask for it in v1.

**Cost.** Finding a document by its contents rather than its name means falling back to
`ripgrep` in the terminal.

---

## D-10 — Platform scope: **macOS only for v1**

**Decision.** macOS is the sole supported platform.

**Rationale.** It is the only platform the user runs, and the Finder integration and single-
instance activation are inherently platform-specific. Supporting one platform properly beats
supporting three poorly.

**Cost.** Some platform-specific work (file association, app activation) will need redoing for
Linux. Kept explicitly in mind so it stays isolated rather than spreading.

---

## D-11 — External change handling: **silent reload when clean, banner when dirty**

**Decision.** When a file open in medd changes on disk, medd compares the in-app buffer against
the last content it wrote. If the buffer has no unsaved edits, the new contents are loaded
silently and the preview updates. If the buffer *does* have edits that have not yet reached
disk, medd stops autosaving that buffer and shows a non-blocking banner above the document —
*"changed on disk — Reload / Keep mine"* — leaving the choice with the user.

**Rationale.** Autosave (D-5) makes this the one place where medd can destroy work, so it is
worth the asymmetry. The clean case is the overwhelmingly common one — the user saved in Neovim,
or switched branches, and medd is simply behind — and prompting there would train the user to
dismiss the banner without reading it, which is exactly how the dirty case gets ignored too. The
dirty case is rare and genuinely ambiguous, and is the only case where a prompt earns its
interruption.

**Rejected: always prompt.** More predictable, but it spends the user's attention on the case
where nothing is at stake, which devalues the prompt in the case where something is.

**Rejected: always reload, disk wins.** Simplest, and defensible if medd were only a viewer —
but medd is an editor, and silently discarding typed text is the worst available failure.

**Cost.** Autosave must be suspendable per buffer, and medd must track "what I last wrote" per
open file rather than just "is this dirty" — the comparison is against medd's own last write,
not against a timestamp, so that medd's own autosave does not trip its own detector. A `Diff…`
affordance on the banner is desirable but not v0.1.

---

## D-12 — WYSIWYG: **not a goal; do not pay for optionality**

**Decision.** WYSIWYG editing stays on the "later, uncommitted" list, but the architecture does
**not** carry an abstraction layer whose purpose is to make the editor swappable. The source
pane is built directly against its editor component.

**Rationale.** The user was asked directly and answered that source + preview is genuinely what
they want; WYSIWYG is a nice-to-have they would probably never build. Research (see
[research/q9-editor-component.md](research/q9-editor-component.md)) independently found that no
seam makes that swap cheap anyway — a rich-text engine's data model is a document tree, a code
editor's is a text buffer, and translating between them is the whole cost. Paying for an
abstraction that would not actually pay out is the worst of both.

**Note.** This is *not* licence to let editor internals leak everywhere. Autosave, external-change
detection, and tab state talk to plain text and a dirty flag — not to editor-specific types —
because that is ordinary good layering, not WYSIWYG insurance.

**Cost.** If WYSIWYG is ever genuinely wanted, the source pane is a rewrite. Accepted knowingly.

---

## D-13 — Repository: **public on GitHub, GPL-3.0**

**Decision.** The repository is public from the first commit and licensed GPL-3.0.

**Rationale.** The user's explicit choice. Copyleft keeps derivatives open; the project has no
commercial ambition that a permissive licence would serve.

**Cost.** Rules out proprietary reuse, including by the author. Contributions, should any arrive,
carry GPL terms. Design documents land in the repository before code, per the process requirement
in requirements.md §5.

---

## D-14 — Workspace switching: **Open Folder… plus recent workspaces, with a welcome empty state**

**Decision.** The workspace root is changed from inside the app via an *Open Folder…* action
(native macOS folder picker, Cmd+Shift+O) or by choosing from a recent-workspaces list. Launching
with nothing to restore shows a welcome pane: the app name, an *Open Folder…* button, and the
recent list. The CLI (`medd ~/docs`) remains an equally first-class way to switch.

**Rationale.** The user chose the conventional, discoverable option over a CLI-only one. It costs
little — a folder picker is a platform dialog, and the recent list is already implied by `medd`
with no arguments restoring the last workspace — and it means the app is usable by someone who
has just double-clicked it from Finder with no terminal in sight.

**Rejected: CLI-only switching.** Fewer moving parts and a fair fit for the user's habits, but it
leaves a bare launch staring at a window with no way forward except going back to a terminal.

**Cost.** A welcome/empty state is a real UI surface to design and build, and the recent-workspaces
list needs persistence (see architecture.md, application state).

---

## D-15 — Opening a file from outside the workspace: **loose tab, tree unchanged**

**Decision.** Opening a `.md` file that lies outside the current workspace root opens it as a tab.
The file tree and the workspace root are untouched. If there is no workspace at all, the file's
parent directory becomes the workspace.

**Rationale.** Resolves Q-3, which was raised for the user but is a tactical call. Re-rooting the
workspace because someone double-clicked a file in `~/Downloads` would throw away the tree and
tabs they were working with — a large, surprising side effect from a small action. A loose tab is
the least destructive reading of the request, and the tab's title bar can show the outside-workspace
path so the user is not confused about what they are editing.

**Ordering dependency (2026-09-16): the re-rooting must happen *before* the document is read, or
P-3 silently fails for that document.** External-change detection for a loose file is attached only
when a workspace is open — so a loose document opened with no workspace gets no watch and no
detection at all. P-3 is a **Must**.

This is unreachable today, because the file tree is the only way to open anything and it requires a
workspace. It stays unreachable *only if* the second clause above — the file's parent directory
becomes the workspace — is applied before the read. A launch path that routes and opens first would
break a Must without anything failing. An ordering dependency rather than a divergence, and
cheapest to state before the CLI lands.

**Cost.** Relative links and images inside a loose file resolve against *that file's* directory,
not the workspace root — which is correct, but means link resolution cannot assume every open
document lives under the root.

---

## D-16 — Document outline: **stays deferred**

**Decision.** A table-of-contents / outline panel remains on the uncommitted "later" list. It is
not pulled forward into v0.1, v0.2, or v0.3.

**Rationale.** Resolves Q-5. It is a genuinely good fit for the reading-mode half of the product,
and it is cheap given that the renderer already walks the heading structure — but the v0.1 bar is
a thin vertical slice, and nothing about deferring it makes it harder later. It is the strongest
candidate to promote once v0.3 lands.

**Cost.** Long documents are navigated by scrolling until then.
