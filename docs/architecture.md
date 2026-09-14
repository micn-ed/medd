# medd — Architecture

**Status:** Draft v1 — architecture phase, 2026-09-14
**Inputs:** [requirements.md](requirements.md), [decisions.md](decisions.md) (D-1…D-16),
[scope-mvp.md](scope-mvp.md), [research/](research/)
**Stage:** Architecture. No implementation code has been written.

This document describes how medd is built. It takes the product decisions in
[decisions.md](decisions.md) as given and does not relitigate them. Where it makes a technical
choice with real alternatives, that choice is recorded as an ADR in [adr/](adr/) and referenced
from here rather than argued inline.

---

## 1. The shape of the system

medd is one process containing two runtimes: a Rust core and a WebView frontend, bridged by
Tauri's IPC (D-2). The division of labour is not arbitrary, and getting it right is the single
most consequential thing in this document.

**Rust owns the filesystem and the process.** Reading files, writing them atomically, watching
them for external change, enumerating the workspace tree, persisting application state, binding
the single-instance socket, and receiving the operating system's file-open events. These are
exactly the jobs where Rust's guarantees pay for themselves, and they are all off the keystroke
hot path.

**The frontend owns the document.** The text of an open document lives in the WebView, because
that is where the editor component runs and where keystrokes originate. Markdown parsing, syntax
highlighting, sanitisation, and rendering all happen there too — see
[ADR-001](adr/001-markdown-rendering.md). The consequence worth stating plainly: **the IPC bridge
is not in the render path at all.** A keystroke never crosses it. Only three things cross it —
loading a document, autosaving a document, and being told a document changed underneath us.

That asymmetry is what makes the latency requirement (N-3) achievable without cleverness. It also
means the two halves have a clean contract rather than a shared mutable model, which is the only
version of a two-language application that stays maintainable.

```
┌─────────────────────────── one process ───────────────────────────┐
│                                                                    │
│  ┌──────── Rust core ─────────┐      ┌──── WebView frontend ────┐ │
│  │                            │      │                          │ │
│  │  single-instance socket ───┼──┐   │  CodeMirror 6 source pane│ │
│  │  RunEvent::Opened  ────────┼──┤   │  markdown-it → preview   │ │
│  │                            │  │   │  tabs · tree · quick-open│ │
│  │  route_open() ◄────────────┼──┘   │                          │ │
│  │  document read / CAS write │      │  autosave debounce       │ │
│  │  fs watcher (notify)       │◄────►│  dirty / conflict state  │ │
│  │  workspace tree            │ IPC  │                          │ │
│  │  state.json · settings.json│      │                          │ │
│  └────────────────────────────┘      └──────────────────────────┘ │
└────────────────────────────────────────────────────────────────────┘
```

### Technology choices

| Layer | Choice | Where decided |
|---|---|---|
| Core language | Rust | D-1 |
| GUI shell | Tauri v2 (macOS WKWebView) | D-2 |
| Source editor | CodeMirror 6 | [ADR-002](adr/002-editor-component.md) |
| Markdown → HTML | markdown-it + GFM plugins | [ADR-001](adr/001-markdown-rendering.md) |
| Syntax highlighting | highlight.js, curated language subset | [ADR-001](adr/001-markdown-rendering.md) |
| HTML sanitisation | DOMPurify | [ADR-001](adr/001-markdown-rendering.md) |
| Single-instance IPC | `tauri-plugin-single-instance` + `RunEvent::Opened` | [ADR-003](adr/003-launch-routing.md) |
| Filesystem watching | `notify` (FSEvents backend) | §6 |
| Frontend framework | Svelte 5 | §10 |

---

## 2. Module layout

```
medd/
├── Cargo.toml                  workspace manifest
├── src-tauri/
│   ├── tauri.conf.json
│   └── src/
│       ├── main.rs             app setup, plugin registration, RunEvent handling
│       ├── routing.rs          route_open(), the pending-open buffer
│       ├── workspace.rs        root, tree enumeration, path classification
│       ├── document.rs         read, atomic write, content hashing, compare-and-swap
│       ├── watcher.rs          notify wrapper, event coalescing, own-write suppression
│       ├── state.rs            settings.json + state.json load/save
│       ├── commands.rs         the #[tauri::command] surface (§4)
│       └── error.rs            one error type, serialised to the frontend
├── src/                        frontend (TypeScript + Vite + Svelte)
│   ├── main.ts
│   ├── editor/                 CodeMirror setup, keymap, theme, find & replace
│   ├── render/                 markdown-it pipeline, highlight, sanitise, link rewriting
│   ├── tabs/                   tab model, per-tab EditorState retention
│   ├── tree/                   sidebar file tree
│   ├── quickopen/              Cmd+P fuzzy matcher and dialog
│   ├── doc/                    dirty tracking, autosave debounce, conflict state machine
│   └── styles/                 reading typography, light and dark
├── scripts/medd                the CLI shim (§8)
└── docs/                       this directory
```

One Rust module per concern, each with a narrow public surface; `commands.rs` is the only module
that knows Tauri's command macros exist, so the rest stays plain Rust and directly unit-testable
(§9).

---

## 3. The document lifecycle

This is the heart of the design, because autosave (D-5) plus external-change detection (D-11)
plus atomic writes (P-4) interact, and each one is easy to get subtly wrong on its own.

### Content hashes, not timestamps

Every document medd has read or written is tracked in Rust by the **hash of its content as medd
last saw it on disk** — not by mtime. Timestamps are coarse, clock-dependent, and change for
reasons unrelated to content; a hash answers the only question that matters, which is *"is what
is on disk now what I think is there?"*. The hash is cheap: these are documents, not disk images,
and it is computed on a read or write we were already doing.

Rust keeps a map `path → last_known_hash` for every open document, guarded by a mutex.

### Writing: compare-and-swap, atomically

Autosave is debounced in the frontend (~1s after typing stops, per P-1) and then calls
`document_write(path, content, expected_hash)`. Rust:

1. Takes the per-path lock.
2. Re-reads the file and hashes it. If that hash is not `expected_hash`, the write is **rejected**
   with a `Conflict` containing the current disk content — someone else got there first. Nothing
   is written.
3. Otherwise writes `content` to a temporary file in the **same directory** (`.medd-<name>.tmp`),
   `fsync`s it, copies the original's permissions onto it, and `rename()`s it over the target.
   Same directory matters: `rename` is only atomic within a filesystem.
4. Records the new content's hash as `last_known_hash` **before releasing the lock**, so that the
   watcher event this write is about to generate can be recognised as our own.
5. Returns the new hash, which the frontend adopts as its `expected_hash` for the next write.

Step 2 is what makes autosave safe rather than merely convenient. Without it, the window between
detecting an external change and the next debounce firing is a lost-update race; with it, medd
physically cannot overwrite a change it has not seen.

### Reading and external change

The watcher (§6) reports that a file changed. Rust re-reads and hashes it:

- **Hash equals `last_known_hash`** → this is medd's own write echoing back. Discarded silently.
  This is the common case and it must be cheap.
- **Hash differs** → a genuine external change. Rust updates `last_known_hash`, and emits
  `document:changed-on-disk { path, content, hash }` to the frontend with the new content
  attached, so the frontend needs no follow-up round trip.
- **File is gone** → emits `document:removed-on-disk { path }`. The tab stays open holding its
  text, marked as detached from disk; autosave for it is suspended until the user saves it
  somewhere. Deleting a file out from under an editor should never silently recreate it.

The frontend then applies D-11:

```
                       document:changed-on-disk
                                 │
                    ┌────────────┴────────────┐
             buffer clean                buffer dirty
      (text === lastSyncedText)     (unsaved edits present)
                    │                          │
        replace buffer contents        suspend autosave
        preserve cursor/scroll         show conflict banner
        adopt new hash                 ┌───────┴────────┐
        no UI shown                 Reload           Keep mine
                                       │                 │
                              take disk content   adopt new hash as
                              discard local edits expected_hash, resume
                                                  autosave (next write
                                                  overwrites disk)
```

"Keep mine" deliberately does not write immediately — it adopts the new disk hash as the
compare-and-swap baseline and resumes normal autosave, so the user's next keystroke is what
commits the decision. `Diff…` is a desirable third option on that banner and is explicitly **not**
v0.1.

### Dirty state

The frontend holds, per open document, the text it last successfully synced with disk
(`lastSyncedText`) and the hash that text has (`expectedHash`). `dirty` is simply
`currentText !== lastSyncedText`. There is no separate dirty flag to fall out of sync — a derived
value cannot lie. Under autosave this is true only for the second or so after typing stops, which
is exactly the window D-11 cares about.

---

## 4. The IPC surface

Deliberately small. Everything else is internal to one side or the other.

**Commands (frontend → Rust)**

| Command | Purpose |
|---|---|
| `frontend_ready() -> Vec<PendingOpen>` | Frontend signals it can receive events; drains the pending-open buffer (§5) |
| `workspace_open(path) -> WorkspaceInfo` | Set the workspace root, start watching it |
| `workspace_pick() -> Option<PathBuf>` | Native folder picker (D-14) |
| `workspace_recents() -> Vec<PathBuf>` | Recent workspaces for the welcome pane (D-14) |
| `dir_list(path) -> Vec<TreeEntry>` | One level of the tree, lazily |
| `document_read(path) -> { content, hash }` | Open a document; begins tracking it |
| `document_write(path, content, expected_hash) -> Result<{hash}, Conflict>` | Autosave, compare-and-swap (§3) |
| `document_close(path)` | Stop tracking; drop a loose-file watch |
| `open_external(url)` | Hand an http(s) link to the system browser (R-6) |
| `state_save(state)` | Persist session state, debounced (§7) |

**Events (Rust → frontend)**

| Event | Payload |
|---|---|
| `document:changed-on-disk` | `{ path, content, hash }` |
| `document:removed-on-disk` | `{ path }` |
| `tree:changed` | `{}` — coalesced; frontend re-lists what is expanded |
| `open:request` | `{ paths: [...] }` — a launch wants these opened (§5) |

Paths crossing this boundary are always **absolute and canonicalised** by Rust. The frontend never
constructs a filesystem path; it echoes back paths it was given. That single rule removes an
entire category of path-handling bugs and makes the security posture trivial to state.

---

## 5. Launch and routing

Full reasoning in [ADR-003](adr/003-launch-routing.md). The architecturally important finding from
research is that the three launch paths **cannot** converge at the transport layer, because macOS
delivers them differently:

- **CLI and Neovim** (D-6 has `:Medd` call the same entry point as the CLI) arrive as `argv` to a
  second process, which `tauri-plugin-single-instance` forwards over a Unix domain socket to the
  running instance and then exits.
- **Finder double-click and "Open With"** never produce a second process at all. macOS routes them
  to the *already running* app as Apple Events, surfacing in Tauri as `RunEvent::Opened`.

So the design is **two listeners, one router**. Both terminate in a single Rust function:

```rust
fn route_open(paths: Vec<PathBuf>) {
    // canonicalise; classify each as workspace-relative, loose file, or directory
    // if the frontend is ready: emit open:request
    // otherwise: push onto the pending-open buffer
    // then, best-effort: window.show(); window.set_focus();
}
```

Two properties of that function are load-bearing:

**The pending-open buffer.** `RunEvent::Opened` can fire before the WebView has attached its event
listeners — this is the documented behaviour, not an edge case. A fire-and-forget event would be
dropped, and the user's double-clicked file would silently fail to open. So opens are buffered in
Rust state and drained by the frontend's `frontend_ready()` call. Cold launch from Finder goes
through the buffer every single time; this is the normal path, not the exceptional one.

**Window activation is best-effort.** `set_focus()` is unreliable on macOS — the underlying
`NSRunningApplication.activateWithOptions` has been flaky since Big Sur and is deprecated as of
Sonoma, with two open Tauri issues tracing to it. No IPC choice fixes this. medd therefore
attempts activation and does not depend on it: **the file opens as a tab whether or not the window
comes forward.** Nothing in the design may assume the window is frontmost after a routed open.
This should be spot-tested early against real window states.

`open:request` handling in the frontend, per path: a directory becomes the workspace; a `.md` file
under the current root opens as a tab; a `.md` file outside it opens as a loose tab with the tree
untouched (D-15); if there is no workspace yet, a loose file's parent directory becomes the root.

---

## 6. Filesystem watching

The `notify` crate, which on macOS uses FSEvents. This matters for scope: FSEvents is a
kernel-level recursive facility, so watching a large tree costs approximately what watching a
small one costs — unlike Linux's inotify, which needs a descriptor per directory. Recursive
watching of the workspace root is therefore affordable here in a way it would not be on Linux, and
that asymmetry is noted now so the Linux port (N-4) treats it as a known divergence rather than a
surprise.

**What is watched:**
- The workspace root, recursively — feeds `tree:changed` and picks up external edits to any open
  document inside the root.
- The parent directory of each open *loose* document (D-15), non-recursively, since those live
  outside the root.

**Event handling:** raw events are coalesced over a ~100ms window and filtered before anything
else happens — paths not under a watched scope are dropped, and `.git/`, `node_modules/`, and
dotfile directories are ignored entirely. Surviving events split two ways: a change to a
*tracked open document* goes through the hash comparison in §3; anything else collapses into a
single `tree:changed`, and the frontend re-lists only the directories it currently has expanded.
The frontend never receives a per-file tree event, so a `git checkout` touching two thousand files
produces one message, not two thousand.

The watcher runs on its own thread and communicates over an mpsc channel drained by a Tauri-managed
task, so filesystem event storms cannot block command handling.

Live tree updates are a v0.3 item (W-6). The watcher exists in v0.1 anyway, because
external-change detection (P-3) is a Must and needs it; `tree:changed` is simply not yet wired to
the sidebar.

---

## 7. Application state on disk

`~/Library/Application Support/medd/`, resolved through Tauri's path API rather than hardcoded.
Two files, deliberately separate:

**`settings.json` — the user's, human-editable.** Autosave delay, theme preference, font size.
Written only when the user changes a setting. A human may edit it by hand and medd will not
clobber it.

**`state.json` — the app's, rewritten constantly.** Last workspace, recent workspaces (capped at
10), open tabs, active tab, sidebar visibility, tree expansion. Saved on a ~2s debounce after any
change, and on quit.

Keeping them apart is the whole point: session state churns every few seconds, and mixing
hand-editable preferences into a file the app rewrites on a timer guarantees that hand edits are
eventually lost. Both are JSON, both are written through the same atomic write path as documents
(§3) — a crash during a state save must not produce a truncated file that stops the app launching.

**Corrupt or unreadable state is not fatal.** A file that fails to parse is renamed to
`.bak` alongside, a fresh default is used, and the app launches. An editor that refuses to start
because its session file is malformed is a worse failure than a lost session.

Session restore itself is v0.2; v0.1 writes and reads the last workspace and recent list only
(the CLI's bare-`medd` behaviour and D-14's welcome pane both need it).

---

## 8. Memory, latency, and large documents

### Memory (N-1, I-3, resolves Q-11)

The requirement said "no unbounded growth" without a number, which is untestable. Concretely:

- **Target:** under 150 MB resident with a workspace open and a handful of tabs. The WKWebView
  baseline is 50–80 MB of that and is not medd's to reduce.
- **Testable stability criterion:** resident memory after eight hours with ten tabs open does not
  exceed resident memory at the five-minute mark by more than 10%.
- **Closed tabs free everything.** No buffer cache, no "recently closed" retention of contents.
- **Only the active tab has a mounted editor view and a rendered preview DOM.** Inactive tabs
  retain their CodeMirror `EditorState` — which is text plus undo history, and is cheap — but not
  an `EditorView`. CodeMirror 6 separates state from view precisely so this is possible, so this
  costs nothing idiomatic and bounds memory at O(total text) rather than O(tabs × editor
  machinery). Switching tabs remounts a view against retained state, which is fast and, critically,
  **preserves undo history**.
- **The tree holds paths and names, never contents.**

No cap on open tab count. With the above, tabs are cheap enough that a cap would be arbitrary.

### Large documents (resolves Q-4)

markdown-it re-parses the whole document per debounce tick rather than incrementally, so preview
cost is linear in document size. Two thresholds, defined as constants in one place so they can be
tuned against measurement rather than argued about:

| Size | Behaviour |
|---|---|
| < 1 MB | Normal: live preview on the autosave debounce |
| 1–10 MB | Opens normally, editing is live, but the **preview switches to manual refresh** — a button, not a keystroke-driven re-render |
| > 10 MB | Opens **read-only, source only, no preview**, with a banner explaining why |

CodeMirror's own large-document handling (viewport-based rendering) is the reason the editing side
stays live where the preview cannot. These numbers are estimates and should be replaced with
measured ones during v0.1; they exist now so that "degrades gracefully" is a specified behaviour
rather than an aspiration.

### Cold start (N-2)

Under one second to a usable window. The budget is dominated by WebView startup and frontend
bundle parse, which is the direct reason CodeMirror 6 was chosen over Monaco
([ADR-002](adr/002-editor-component.md)) and why highlight.js ships a curated language subset
rather than every grammar. The workspace tree is enumerated lazily, one directory level at a time —
a workspace with ten thousand files must not be walked before the window appears.

---

## 9. The CLI and its relationship to the app bundle (resolves Q-14)

`medd` on `PATH` is a small shell shim, symlinked from `/usr/local/bin` (or `~/.local/bin`) by a
`make install-cli` target. Homebrew packaging is deferred.

The shim exists because the warm and cold cases genuinely differ:

- **An instance is running.** The shim executes the app binary inside the bundle
  (`Medd.app/Contents/MacOS/medd`) with the user's arguments. The single-instance plugin's client
  side detects the bound socket, forwards `argv` and the working directory, and exits in
  milliseconds. Going through the plugin's own client rather than writing to the socket directly
  keeps medd off the plugin's internal wire format.
- **No instance is running.** The shim uses `open -a` so the app is launched by Launch Services —
  detached from the terminal, with proper bundle identity and dock presence. Executing the binary
  directly here would tie the app's lifetime to the terminal session that started it, which is
  plainly wrong for a resident app.

The shim distinguishes the two by testing whether the single-instance socket accepts a connection.
That socket path must be derived from a fixed, stable location (under the application support
directory) computed identically by both the shim and the app — never from the invoking binary's
own location, since the shim and the bundle do not know where the other lives.

The shim resolves relative paths to absolute before handing them on, because the running instance's
working directory is not the user's.

This is the fiddliest part of the design and the most likely to need adjustment once it meets a
real machine. The constraint that must survive any adjustment is the stable shared socket path.

---

## 10. Frontend framework

**Svelte 5.** The frontend is small — a tab bar, a file tree, a modal, two panes, a banner — but
"small" is exactly where hand-rolled DOM state management rots first, and the tree and tab models
have real state. Svelte compiles to direct DOM operations with a minimal runtime, which serves the
cold-start budget better than React or Vue, and it stays out of CodeMirror's way (CM6 manages its
own DOM subtree and must not be reconciled by a virtual DOM).

This is the one choice here not backed by the research pass, and it is deliberately low-stakes:
no product decision depends on it, and the UI surface is small enough that revisiting it would be
an afternoon rather than a rewrite.

---

## 11. Rendering pipeline

Detail in [ADR-001](adr/001-markdown-rendering.md); the pipeline shape is:

```
CodeMirror text ──debounce──► markdown-it (GFM plugins)
                                   │
                                   ├─ fenced code ──► highlight.js (curated languages)
                                   ├─ link rewriting ──► classify: workspace .md | loose .md
                                   │                              | anchor | external
                                   └─ image src rewriting ──► asset: protocol, resolved
                                                              against the document's directory
                                   ▼
                              DOMPurify ──► preview DOM
```

Three parts of that are medd-specific rather than off-the-shelf:

**Link classification (R-2, R-6).** A markdown-it renderer rule rewrites every link at render time
rather than intercepting clicks afterwards, so the classification is visible in the DOM and
testable without a browser. `http(s)` links become real external links handled by `open_external`;
relative `.md` links become internal links carrying a resolved absolute path, which opens a tab;
in-document anchors scroll.

**Image resolution (R-4).** Relative image paths resolve against **the document's own directory**,
not the workspace root — which is what makes loose files (D-15) render correctly. The rewritten
`src` uses Tauri's asset protocol, scoped to the workspace root and to the directories of open
loose documents; nothing else is readable by the WebView.

**Typography (R-7).** The preview stylesheet is the product, not decoration — D-3's reading mode
exists because reading is the dominant use. Measure, vertical rhythm, heading scale, table
borders, and code-block treatment get deliberate attention in light and dark, and live in one
stylesheet that reading mode and split view share.

---

## 12. Testing strategy (resolves Q-15)

The bar is: **anything that can silently lose a user's data is tested automatically; anything that
is a matter of taste or a platform quirk is verified by hand.**

**Rust unit tests** — the data-loss surface, and the reason `commands.rs` is the only Tauri-aware
module:
- Atomic write: after a simulated crash between temp-write and rename, the original file is intact
  and unmodified. Permissions are preserved across the rename.
- Compare-and-swap: a write with a stale `expected_hash` is rejected and touches nothing.
- Own-write suppression: a write followed by its watcher event produces no change notification.
- External change: a foreign write produces exactly one notification with correct content.
- Path classification: workspace-relative, loose, directory, symlink, non-existent.
- State files: malformed JSON is backed up and defaults are used; the app still starts.

**Frontend unit tests (vitest)** — the correctness surface:
- **Golden-file rendering tests.** A corpus of `.md` inputs with expected HTML fragments covering
  GFM tables, task lists, footnotes, strikethrough, fenced code, images, and nested emphasis. This
  is the regression net for R-1…R-7 and the thing that makes a future parser swap (ADR-001's
  named escape hatch) a measurable change rather than a leap of faith.
- Link classification: each of the four link kinds resolves to the right target.
- Image path resolution, including a loose document outside the workspace root.
- The **autosave and conflict state machine** (§3, D-11) under fake timers: debounce firing, clean
  reload, dirty banner, Reload, Keep mine, rejected-write-becomes-conflict. This logic is small,
  stateful, and the most dangerous code in the frontend; it deserves exhaustive tests.

**Verified by hand:**
- Window focus on a second launch — known unreliable on macOS (§5), so it is checked, not asserted.
- macOS keybinding fidelity, IME behaviour, and clipboard inside WKWebView, which differ from a
  browser tab and cannot be trusted from browser-based assumptions.
- Reading typography in light and dark.
- Large-document thresholds (§8), which is where the estimated numbers get replaced by measured
  ones.

**Not in v0.1:** end-to-end driving of the built app. `tauri-driver` exists, but it is a real
investment, and the thing most worth driving end to end — three launch paths converging on one
instance — is a v0.2 deliverable. Revisit it then.

---

## 13. What this architecture deliberately does not do

- **No Markdown parsing in Rust.** Nothing outside the WebView needs parsed Markdown in v0.1. A
  future full-text search index would be the first real reason to add it, and it would be additive.
- **No incremental rendering.** Full re-parse per debounce tick, with size thresholds (§8) instead
  of an incremental pipeline. Incremental rendering is a large amount of machinery for a problem
  that thresholds handle adequately at the document sizes this product targets.
- **No editor abstraction layer.** Per D-12, the source pane is built directly against
  CodeMirror 6. Autosave and change detection talk to text and a dirty flag rather than editor
  types, which is ordinary layering, not swappability insurance.
- **No daemon, no second process, no multiple windows.** D-7.
- **No plugin system, no sync, no accounts.** Out of scope permanently.

---

## 14. Known risks

| Risk | Where | Posture |
|---|---|---|
| `set_focus()` unreliable on macOS | §5, ADR-003 | Accepted; activation is best-effort by design. Spot-test early. |
| markdown-it GFM edge-case fidelity | ADR-001 | Accepted; golden tests pin actual behaviour. comrak is the named escape hatch. |
| CodeMirror 6 ramp-up cost | ADR-002 | Real; budget learning time for the extension model rather than treating CM6 as a drop-in widget. |
| WKWebView clipboard/IME quirks | §12 | Smoke-test against the real WebView early, not against browser assumptions. |
| Large-document thresholds are guesses | §8 | Stated as tunable constants; replace with measurements during v0.1. |
| CLI shim warm/cold detection | §9 | Fiddliest part of the design. Stable shared socket path is the invariant that must hold. |
