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

One Rust module per concern, each with a narrow public surface.

**Tauri types are confined to a named, minimal set of shells: `commands.rs`'s command surface and
`watcher.rs`'s emit loop. A shell contains no decisions. If a Tauri-aware function has a branch in
it, it is in the wrong place.**

**The workspace predicates.** Every question about what medd considers part of a workspace is
answered exactly once, in `workspace.rs`: `is_markdown`, `is_ignored_name`, `resolves_to_directory`.
Together these three *are* medd's definition of a workspace's contents — not helpers that happen to
share a file. **A caller that computes one of these answers again is a bug, even when it computes
the same thing.** That last clause is the operative one: every instance of this on the project has
been a new consumer re-deriving an answer that already existed, and at least once the duplicate was
character-for-character identical to the shared function, which is exactly why "they agree today"
is not a defence.

They are *predicates*, deliberately, not a classifier. A shared `classify_entry -> EntryKind` was
considered and rejected: `dir_list` asks a three-way presentational question (W-8 requires it to
categorise non-Markdown files as `Other`), while the walk asks two-way questions and has no use for
a sidebar category it must then ignore. That is coupling, not sharing. The shared thing is the atom
they genuinely have in common.

The naming carries the ruling. `resolves_to_directory`, not `is_directory` — the failure mode is
someone simplifying it to `entry.file_type().is_dir()`, and a name containing *resolves* makes that
substitution visibly wrong at the call site.

**And no lock is held across a filesystem or OS call.** Every command was `ExecutionContext::
Blocking` until increment 9, so commands could not overlap and a lock held across a syscall blocked
nobody — there was nobody to block. The first `async` command makes that assumption false, and it
makes two pre-existing sites real on the day it lands rather than at some later point: one holding
the workspace guard across canonicalisation and an FSEvents registration, one holding *both* the
workspace and watcher guards across a recursive registration over an entire tree. Copy what is
needed out of the guard and release it before touching the filesystem — which is what
`run_event_loop` already does, and the reason none of this was ever visible: the only concurrent
actor in the system happened to be doing the right thing.

Two constraints that are currently true by luck and should be true by rule. **Lock order is
workspace → watcher**, consistently, so no AB/BA deadlock is waiting; say so where the second lock
is taken, because the next person to add a lock site has nothing else telling them an order exists.

That claim is checked rather than observed. Every site was enumerated: only `workspace_open` and
`document_read` take both, both in that order; `dir_list` takes the workspace lock alone; `quit.rs`
holds three independent mutexes, never one across another. One pair looks like a reversal and
isn't — `run_event_loop` takes workspace → `last_known`, and `document_read` takes `last_known` →
workspace, but **sequentially rather than nested**: `store.read()` returns and releases before the
workspace lock is acquired. Worth recording precisely, because the next reader will see the
apparent reversal and needs to know it was examined rather than missed.
And note that on **edition 2021 an `if let` scrutinee temporary lives for the whole body** — which
is why one of those guards is held at all. Edition 2024 changes that, so an edition bump would
silently fix this *and* silently change anything else relying on scrutinee temporary lifetimes.

**Violations cluster here for a structural reason, not by accident**, and knowing why predicts
where the next one will be: **the shell is the only place with access to everything at once** — the
handle, the managed state, the request — so it is exactly where it is most convenient to do work
that needs two of them. That convenience is the force this rule exists to resist.

Three instances so far, each with the same tell — *the untestable thing was untestable because of
where it lived*: the watcher's event loop holding the `AppHandle`; the quit repeat-press decision
inside `main.rs`'s `RunEvent` match; and two command handlers holding locks across OS calls. In
every case the fix was to move the decision out, never to build a harness that could reach it.

And the fix has a stronger and a weaker form. Moving a lock guard's release earlier leaves the
invariant *currently satisfied*, able to drift back with nothing failing when it does. Extracting
the work into a plain function that receives what it needs already copied out makes the invariant
**impossible to violate at that site** — a function cannot hold a guard it was never given. Prefer
the structural form, for the same reason a derived value beats a stored one.

**And extraction is only structural if the *signature* carries the guarantee.** Here that means an
owned `PathBuf`, not `&Path`: `ws.root()` borrows out of the `Workspace`, which borrows out of the
`MutexGuard`, so a borrowed parameter forces the guard to stay alive for the whole call.
Demonstrated by construction — with `&Path` the lock is unavailable to another thread during the
call; with `PathBuf` it is available.

Worth stating explicitly because **`&Path` is the idiomatic signature**. Preferring a borrow over
an allocation is ordinary good advice, so it is what a careful implementer writes and a careful
reviewer approves — and it leaves the invariant exactly as violated while *looking* addressed,
which is worse than leaving it alone, because nobody re-checks something that appears handled. The
allocation is the mechanism, not a cost to optimise away.

The general form: extraction with a permissive signature *moves* a violation rather than removing
it, and consumes the attention that would otherwise have found it. A call site that copies today
can be edited back tomorrow with nothing failing; a signature that only accepts owned data cannot
be, without the change appearing in the diff as a type change.

An earlier version of this rule said `commands.rs` was the only module that knew Tauri's *command
macros* existed. That described an implementation detail and called it a boundary, and it let a
second module become Tauri-aware without tripping it — `watcher.rs` imports `AppHandle`/`Emitter`
and emits inline, and it is no coincidence that the function holding the `AppHandle` turned out to
be the one with no test at all. The property the rule was always buying is that every decision is
reachable without a runtime; the restatement above is checkable by reading, which the original was
not.

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

**The guard stops path *construction* escaping the workspace; it does not stop the user's own
symlinks being followed.** `..` is construction — a caller assembling a path out of the workspace —
and is rejected outright. A symlink is *content*: the user placed it there deliberately, and D-4's
root folder is what they meant by doing so. So the order is: reject any path containing a `..`
component, require it lexically under the root, and only then resolve whatever links are present.
That closes `root().join("../outside.md")` **before** any link resolves, and it is auditable by
reading rather than by reasoning about resolution order.

Following symlinks gives up the type-level cycle immunity that not following them provides, so the
replacement is unconditional rather than careful: a visited set of canonical paths, which is also
correct for diamonds. **Test the diamond, not the cycle** — two links to one real directory
terminates either way and fails as a deterministic count, whereas a cycle test fails by *hanging*,
which is the worst failure mode a suite can have.

**Paths are canonicalised before anything touches disk**, and the tracked key is always the real
file rather than whatever the caller handed in. This is not merely tidiness — it is what makes
writing through a symlink safe. `rename()` unlinks whichever directory entry it is given, so
staging a temp file beside the *symlink* and renaming over it replaces the symlink with a plain
file: the real document never receives the edit, and the link the user set up is destroyed. Worse,
if the symlink and its target sit on different filesystems the rename fails with `EXDEV` and the
write silently does nothing at all. Canonicalising first puts the temp-file-and-rename dance in the
real file's own directory, so the symlink is never touched and keeps pointing at the updated file.

**The mutex covers the whole of a read as well as the whole of a write.** Taking it only at the end
of a read — after the bytes have already been pulled off disk — leaves a window where a concurrent
write can commit a newer hash that the finishing read then overwrites with its older one. The map
would then disagree with disk, and the watcher below would classify medd's own write as an external
change. That is not a data-loss bug; it is a *trust* bug, and a worse one than it sounds: D-11's
entire argument rests on the conflict banner being rare enough to be believed. A banner that
appears after edits nobody made is how users learn to dismiss it unread. Measured on the
implementation, the losing interleaving occurred in 398 of 400 attempts — this is the common case,
not a corner.

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

### Line endings: normalised on read, restored on write

"The frontend owns the document" (§1) has a consequence the original text didn't state: CodeMirror
normalises whatever line breaks it's handed to `\n` internally, regardless of what medd gives it.
Left alone, that means a CRLF document has two different texts in play the moment it opens — the
raw CRLF bytes in `lastSyncedText`, and the same document LF-normalised inside the retained
`EditorState` — and every diff computed between them (in particular the minimal-change reload on
an external edit) is comparing across a boundary neither side knows exists. Increment 7's review
found this corrupts CRLF documents on exactly the path §3 calls the safe, silent one, and the
corruption then autosaves.

The fix, and the invariant it establishes: **the frontend never sees anything but LF, and never
learns line endings exist.** `document_read` normalises to LF-only content before handing it over;
`document_write` restores the file's own convention immediately before the bytes touch disk, and
hashes what was actually written, not what the frontend sent.

**The convention is never stored — it is derived, at each end, from the bytes in hand.** An earlier
version of this section described it as detected on read, kept, and restored on write. The
implementation does something strictly better and the specification is corrected toward the code
rather than the other way round: by the time `document_write` converts, it has already read the
current bytes *and* proved their hash matches what the caller expected, so disk is provably exactly
what medd believes it is — which makes those bytes the authoritative source for the file's
convention at that instant. A derived value cannot go stale; a stored one can, and would have, the
moment `document_close` evicted its entry while a close-flush was still in flight.

This is the same rule as the one governing dirty state, one layer down: **derive, don't store.** It
also means a file whose convention changes externally is simply picked up — `dos2unix` run on a
document medd has open is not silently reverted by medd's next write, and `unix2dos` is adopted.

**`check_external_change` normalises too, and naming it is not redundant.** It is the path the
corruption was actually found through: `document:changed-on-disk` delivered raw CRLF into a diff
against an LF buffer, and neither `document_read` nor `document_write` was ever involved. A
specification naming only read and write is narrower than the fix it describes, and anyone
implementing from it would leave the watcher path out — breaking the invariant on precisely the
path it was written to protect. Only two conventions
exist — LF and CRLF — and a lone `\r` (classic Mac-era files) folds into LF at detection time
rather than being given a third representation. A file with mixed endings is, as a documented
consequence rather than a bug, fully normalised to its dominant convention by its *first* write
through medd — collapsing to one in-memory representation only has one convention left to restore.
That means a byte change to lines the user did not edit, which is a real cost and an inherent one:
it falls directly out of "the frontend never learns line endings exist", and there is no better
answer available at this layer.

The invariant this buys: **`lastSyncedText` and the retained `EditorState` are always in the same
line-ending convention (LF), so a diff between them is always comparing like with like.**

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
                              take disk content   adopt new hash as baseline,
                              discard local edits  schedule an autosave tick
                                                    (next write overwrites disk)
```

**"Keep mine" resumes autosave rather than performing a write of its own.** It adopts the new disk
hash as the compare-and-swap baseline, clears the conflict, leaves the buffer untouched, and
schedules an autosave tick. Because `dirty` is derived (`currentText !== lastSyncedText`) and
`lastSyncedText` becomes the *disk's* content, the buffer is dirty against the new baseline, so
that tick writes the user's version over disk — about a second after the click, with no further
keystroke needed.

What "does not write immediately" buys is a single write path: `resolveConflictKeepMine` is a pure
state transition with no I/O, so there is exactly one place in medd that writes a document, and it
is the one that is compare-and-swapped and tested. Scheduling a tick explicitly (`doc/`'s
`keepMine`), rather than relying on whichever debounce timer happens to have survived the conflict,
is what makes that true in every case rather than most of them — increment 7's review found the
un-scheduled version left "Keep mine" unsaved indefinitely whenever the conflict was discovered by
a rejected compare-and-swap write, since that write had already spent the only timer in flight.

**The shutdown latch is one-way, and that is safe only because nothing cancels a quit.** Once
shutdown begins, autosave scheduling, external-change application and document opening are all
suppressed — drain what is owed, accept no new work. There is no path back, and none is needed,
because the latch is set only once the process is committed to exiting within a bounded time.

**Whoever adds a cancellable quit must clear the latch.** If a quit can be aborted and the latch
survives, autosave is silently off for the rest of the session — the worst-shaped bug this product
can have. That is not hypothetical: the conflicted-tab cost recorded immediately below is exactly
what would tempt someone into a *"you have unresolved conflicts — really quit?"* prompt, and a
prompt implies a cancel. The dependency is recorded here rather than pre-empted with a clearing
function nobody calls, because an abstraction whose only consumer is a hypothetical is the thing
D-12 warns against; a note is what makes the constraint visible to whoever writes the prompt.

**A conflicted tab does not flush when it is closed, or when medd quits — and this is the one
place the product knowingly discards typed text.** The autosave path declines to write while a tab
is in conflict, which is correct: closing a tab or quitting the app must not silently pick a side
in a disagreement the user has been asked to resolve, and prompting would contradict P-2's promise
that closing is always safe. But it means the on-screen banner is the *entire* warning, and a user
who has stopped noticing it loses those edits on quit.

It is recorded here as a decision rather than left as a property nobody named. The cost is real and
is accepted because the alternatives are worse: writing the user's version would resolve the
conflict on their behalf, writing the disk version would discard their edits with even less signal,
and asking would reintroduce the modal save prompt D-5 exists to remove. If anything softens this
later it should be the banner becoming harder to ignore, not the close path becoming cleverer.

The consequence to be honest about: **"Keep mine" discards the disk version within about a second,
and the user has never seen it.** `Diff…` is a desirable third option on that banner and is
explicitly **not** v0.1; with it deferred, the banner's own wording is what tells the user that
before they click, which is why it says what it overwrites rather than just what it discards.

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
The bundle identifier is `com.micned.medd`, and the bundle targets are `app` and `dmg` only,
matching the macOS-only scope (D-10).
Two files, deliberately separate:

**The atomic-write protocol has its own owner, `atomic.rs`.** `pub(crate) fn write(target,
content, new_file_mode)`, called by both `document.rs` and `state.rs`, with the staging-file naming
(`TEMP_PREFIX`, `target_of_temp`, `is_staging_file`) moving with it — those are properties of the
*protocol*, not of documents, which is why the watcher was already importing a staging-file
predicate from `document.rs`.

The alternative — making `document::atomic_write` public — was rejected because it **downgrades a
structural property to a convention.** "No document write bypasses the compare-and-swap" currently
holds because `atomic_write` is private and its only caller CASes first; a `pub` would leave that
true only by everyone remembering. The extraction keeps it structural: the CAS stays in
`DocumentStore::write`, which remains the only function that writes a *document*.

Two consequences that must be decided rather than inherited. **`write` creates an absent target**,
because `atomic_write`'s precondition — that the target already exists, so its permissions can be
copied — is free for a compare-and-swap write that has just read the file, and **false for both
state files on first run.** Left as-is, the very first `state.json` save returns `NotFound`, and so
does the directory above it: Tauri's path API *resolves* Application Support, it does not create
it. Both fail only on a fresh install, which is the one configuration the person building this is
least likely to be in. And **the caller supplies a create-or-refuse policy**, not merely a mode —
`WhenAbsent::Fail` for documents, `WhenAbsent::Create { mode: 0o600 }` for state.

An earlier version of this ruling said "the caller supplies the mode", and that was wrong in a way
a surviving mutant exposed. `DocumentStore::write` reaches the write having *just re-read the
document* for its hash comparison, so an absent target there means the file was **deleted
underneath us** — and creating it would silently recreate a file the user deleted, which §3 forbids
in as many words. The old code refused *by accident*, because it read the target's permissions
unconditionally. **A bare mode parameter would have converted that accident into a recreation.**
The mode reasoning stands; it needed the create-or-refuse decision in front of it.

Flipping the document call site to `Create` kills no test, and that is expected rather than a gap:
the compare-and-swap makes an absent target a microsecond race rather than a reachable state, so
the choice is correct **by construction, not by coverage.** That is recorded at the call site,
because a surviving mutant is otherwise an invitation to decide the distinction does not matter.

**State-file staging litter is never swept, and that is accepted.** The sweep is guarded to
Markdown targets, so `.medd-state.json.tmp` is correctly not medd's business as far as that guard
can tell — which is the `.md` guard's own documented failure mode arriving one increment later, in
the words it was written in. It failed *safe*, which is the better outcome. One stale temp file per
interrupted write, in an invisible directory with no `git status` to surface it, is not worth a
second sweep path.

**Recents and the last workspace are validated on read, not merely parsed.** Both this section and
increment 11 handled a file that fails to parse and neither mentioned one that parses perfectly and
names paths that are gone — which is the **commoner** case: corruption needs a crash mid-write,
staleness needs only time. A project renamed, a clone deleted, a drive unmounted. So: drop recents
whose directory no longer exists, and treat a dead last workspace as *no workspace*, which lands on
the welcome pane — exactly where the user wants to be. And **the cap is enforced on read as well as
write**: ten thousand recents in a hand-edited file parses fine and takes the success path.

**The corruption path's own failure must also be non-fatal.** A naturally written `fs::rename(…)?`
for the `.bak` propagates and kills startup — precisely what this rule exists to prevent. Defaults
are used *regardless of whether the backup succeeded*.

**`settings.json` — the user's, human-editable.** Autosave delay, theme preference, font size.
Written only when the user changes a setting. A human may edit it by hand and medd will not
clobber it.

**`state.json` — the app's, rewritten constantly.** Last workspace, recent workspaces (capped at
10), open tabs, active tab, sidebar visibility, tree expansion. Saved on a ~2s debounce after any
change, and on quit.

Keeping them apart is the whole point: session state churns every few seconds, and mixing
hand-editable preferences into a file the app rewrites on a timer guarantees that hand edits are
eventually lost.

**The no-clobber guarantee is currently true for a weaker reason than this section implies**, and
that is worth stating before someone relies on it. In v0.1 nothing writes `settings.json` at all —
there is no preferences UI (P-5 is a *Could*), so the only writer is the hand edit. Once a
preferences UI exists, changing one setting writes medd's whole in-memory `Settings` and silently
discards any hand edit made since load. That is D-11's lost-update problem in a second place, and
it is unsolved. Not a v0.1 defect; a sentence that reads as a property of the design when it is a
property of the current feature set. Both are JSON, both are written through the same atomic write path as documents
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

**One path, no probe.** The shim always executes the app binary inside the bundle
(`medd.app/Contents/MacOS/medd`), detached, with the user's arguments. The single-instance plugin
then makes the warm/cold decision itself: if a socket is already bound it forwards `argv` and the
working directory and exits in milliseconds; if not, this process becomes the primary.

An earlier version of this design had the shim probe the socket first and branch — executing the
binary when warm, `open -a` when cold. That was wrong in a way worth recording, because it looks
reasonable: it re-decides something the plugin already decides correctly, atomically, and exactly
once, and the probe itself is not side-effect-free. A bare connect-and-close makes the primary's
read return zero bytes, which becomes an empty argument list, **which fires the primary's open
callback.** Every CLI invocation would have opened a spurious document and attempted activation
twice, and two probes racing would reintroduce a TOCTOU the plugin doesn't have.

**Detachment is doing real work and is not optional.** Measured on a real bundle: executing the
binary directly gives correct bundle identity (`CFBundleIdentifier` resolved from the bundle path),
`type="Foreground"` — so Dock presence and normal activation policy — and, launched with `nohup … &`,
the process reparents to `launchd` and outlives the terminal that started it. Without that
detachment the cold-launch case dies with the shell, which is what `open -a` was originally there
to prevent.

**The socket path is the plugin's, not medd's.** `tauri-plugin-single-instance` hardcodes
`/tmp/<identifier>_si.sock` — `/tmp/com_micned_medd_si.sock` here — and exposes no configuration
hook. This document previously specified a path under the application support directory, which
nothing ever binds; anyone implementing it literally would have built a path that could never be
found. The *invariant* that specification was protecting is intact and is worth restating, because
it is the property that matters: the path derives only from `config.identifier`, a compile-time
constant, and from **nothing about the invoking binary's location** — so the shim and the bundle
never need to agree on where the other lives.

The shim resolves relative paths to absolute before handing them on, because the running instance's
working directory is not the user's.

**`medd <nonexistent-file>` is an error in v0.1**, not a create. Every path canonicalises before
use, so the most natural CLI invocation for a *new* note fails — and that is the accepted
behaviour rather than an oversight. v0.1 has no New File anywhere (that is a v0.3 item), so
create-on-CLI would be the only creation path in the product, arriving through its most obscure
entry point; and a file that does not exist yet has no compare-and-swap baseline, which is exactly
the ambiguity §3 exists to eliminate. The error must say the file does not exist, and must not use
an error surface that replaces the tab UI — hiding the user's open documents because one argument
was wrong is worse than the problem it reports.

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

**Verified in the browser harness (`npm run harness`):** typography, heading scale, tables, code
blocks, the reading-vs-split measure, both themes, the tree, tabs, mode toggling — the frontend
rendered in an ordinary browser against an in-memory fixture workspace. This exists because Tauri
on macOS has no WebDriver (`tauri-driver` is Linux and Windows only), which left the frontend
unverifiable by eye for six increments — during which a CSS specificity bug shipped undetected,
and reading mode's measure ran about 40% over its intended width because `ch` is the advance width
of the "0" glyph rather than of an average character in running prose.

It proves nothing engine-specific: the harness is Blink, the app is WKWebView. A green harness says
nothing about clipboard, IME, or native key handling — those remain hand-verified below.

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
