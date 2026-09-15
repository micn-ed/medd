# Increment 9 — three things quick-open needs that the plan doesn't say

**From:** principal architect
**For:** [plan-v0.1.md](plan-v0.1.md) §9, in flight
**Status:** written before it lands. Measured against medd's own repository at `6576f05`.

§9 is three bullets, and the increment should stay small — that instruction is right and nothing
here argues with it. But one of those bullets is a requirement rather than a design, and the
mechanism it implies does not exist yet:

> The file list is built lazily and cached; **a workspace scan must not block the dialog opening.**

Three things follow from that, ordered by how soon they bite. The first is measured, not argued.

---

## 1. A recursive walk of medd's own repository enumerates 41,875 files to find 25 documents

`workspace::dir_list` filters **dotfiles only**. It does not filter `node_modules`, and it does not
filter `target/`. Measured just now, on this project:

```
all files                       53,165
excluding dotfiles              41,875     <- what a dir_list-based walk would enumerate
  of which node_modules          6,650
  of which target/              45,805
actual .md documents                25     <- what quick-open is for
```

About **1,675 files enumerated per useful result**, in the workspace a medd developer is most
likely to open first. A Rust-plus-Node project is not an unusual shape for a folder of Markdown
notes — it is the normal one, since D-4's whole premise is that the workspace is a project
directory.

**The ignore rules already exist, in the wrong module for this.** `watcher.rs` has
`IGNORED_ANCESTOR_NAMES` and `is_ignored_in_workspace`, which skip `node_modules` and dotfile
directories. `dir_list` has its own, narrower rule. That is two places holding overlapping
knowledge about what medd should ignore — the shape this project has now been bitten by five
times, and the one `is_markdown` was extracted to stop last week.

So: **one shared ignore predicate, in `workspace.rs` beside `is_markdown`**, used by `dir_list`,
by the quick-open walk, and by the watcher. Extracting it is a smaller change than the bug it
prevents, and it is the same move that has already paid off twice.

Two things fall out of doing it there:

- **`target/` should join the list.** It is ignored by neither consumer today. For quick-open that
  is 45,805 files; for the watcher it means every `cargo build` in a medd workspace storms
  `tree:changed`. Harmless now (the storm coalesces to one event and nothing is wired to it),
  a real bite when W-6 lands in v0.3.
- **The tree should probably agree.** `dir_list` currently *shows* `node_modules` in the sidebar,
  which nothing has noticed because nobody has opened a JS project and expanded it. Whether the
  tree hides it is a product call, not mine — but the two behaviours should be one decision rather
  than two accidents.

---

## 2. There is no command that can enumerate the workspace, and a sync one would block the dialog

§4's IPC surface has `dir_list(path) -> Vec<TreeEntry>` — **one level, by design** (increment 3:
"do not walk the tree eagerly"). Quick-open needs every `.md` under the root. Nothing provides
that, and §4 does not list it.

Two ways to build it, and both need saying out loud because the plan implies neither:

- **The frontend walks, via N `dir_list` calls.** One IPC round trip per directory. Tauri commands
  default to `ExecutionContext::Blocking` (verified in `tauri-macros`' wrapper), which runs the
  handler inline on the thread dispatching the IPC message — the main thread on macOS, which is
  also the thread WKWebView runs JavaScript on. So the walk blocks the UI for its duration, which
  is exactly what the bullet forbids.
- **A new Rust command walks recursively.** This is the right answer, and it needs to be
  `#[tauri::command(async)]` for the same reason — a sync one blocks the same thread. That is a
  new entry in §4's IPC surface, and it is the first async command in the project.

Worth flagging because it has a consequence beyond this increment: **an async command runs on the
threadpool, so it can run concurrently with other commands, which sync commands cannot.** Today
every command is serialised on the main thread, and several places quietly rely on nothing else
being able to run — `DocumentStore`'s single mutex is defensive about this and holds, but it is the
first time that assumption gets tested rather than assumed. It is also precisely the change that
would have made the temp-file sweep unsafe had it stayed in `dir_list`, which is why it moved.

---

## 3. The cache has no invalidation signal, and fixing that is nearly free

"Built lazily and **cached**" — with nothing to invalidate it. Create a note in Neovim while medd
is open, press Cmd+P, and it is not there. That is a bug report on day one of using the feature as
intended, since the whole point of medd being resident is that you leave it open.

The signal already exists: `tree:changed` is emitted by the watcher today and deliberately not
wired, because **live tree updates are W-6, deferred to v0.3.** But those are two different things,
and the plan conflates them:

- *Re-rendering the sidebar when the filesystem changes* is UI work, and deferring it is right.
- *Not serving a stale list from a cache* is correctness, and it costs one listener that clears the
  cache.

Wiring `tree:changed` to `quickopen`'s cache invalidation does not pull any of W-6 forward. I would
do it in this increment rather than shipping a cache that is knowingly wrong.

---

## 4. One small thing: what does Enter do on a non-Markdown result?

D-9 says fuzzy quick-open by filename; W-8 says non-`.md` files are visible but inert. If the list
includes them, Enter reaches `document_read`, which returns `NotUtf8` for anything binary — and
`App.svelte`'s top-level `{#if error}` branch **replaces the entire tab UI** with the message,
hiding whatever the user had open. So a stray Cmd+P on a PNG closes your documents from view.

Filter the list with `workspace::is_markdown` and the question does not arise. (The `App.svelte`
error placement is its own bug, already noted against increment 10's
`medd <nonexistent-file>` case — worth fixing once, not twice.)

---

## What I would actually change in §9

Two bullets, not a redesign:

- The file list comes from a **new async command** that walks the root once, filtered by a
  **shared ignore predicate** in `workspace.rs` — `node_modules`, `target/`, dotfile directories —
  and by `is_markdown`. First async command in the project; note the concurrency change.
- The cache is **invalidated by `tree:changed`**, which the watcher already emits. This is not W-6
  and does not pull it forward.

Everything else in §9 stands, including "this increment is small and should stay small".
