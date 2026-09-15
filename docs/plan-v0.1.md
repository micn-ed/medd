# medd — v0.1 implementation plan

**Status:** Ready for implementation, 2026-09-14
**Scope:** the thin vertical slice defined in [scope-mvp.md](scope-mvp.md)
**Built against:** [architecture.md](architecture.md) and [adr/](adr/)

Twelve increments. Each is a coherent, reviewable piece of work with its own definition of done.
They are ordered by **risk, not by appetite**: the code that can silently destroy a user's
document lands early and tested, the one known ramp-up cost lands before it can block anything
else, and the fiddliest platform work lands last because nothing depends on it.

Increments are sequential by default. Where two can genuinely overlap it is noted.

## How increments are handed off

Each increment is committed locally, verified, then pushed.

**Authorship and acceptance are separate.** Whoever writes an increment writes its tests too —
that is how correct code gets written, not a verification step, and in practice it is where
almost every real defect on this project has been caught: a test written alongside the code, then
the code deliberately broken to watch the test fail for the right reason. Acceptance is somebody
else's: an independent pass against the increment's definition of done, adversarial where it can
be, looking for what the author and the reviewer both missed. Neither substitutes for the other,
and collapsing them loses the half that finds things.

Two further conventions make this safe when more than one person is working the same checkout:

- **The working tree belongs to whoever is mid-increment.** Reviews, documentation edits, and
  exploratory work wait for the gap between increments, or happen in a copy outside the repo.
- **Commit explicit paths, never `git add -A`.** A blanket add sweeps up someone else's
  uncommitted work in progress, which at best produces a commit whose message does not describe
  its contents, and at worst loses that work to a later reset. Check `git status` before
  committing and stage only what you changed.

---

## 1 — Skeleton

**Goal.** A Tauri v2 application that builds and runs on macOS, with the frontend toolchain in
place. Nothing else.

- Cargo workspace; `src-tauri` with Tauri v2 and a single window.
- Vite + Svelte 5 + TypeScript frontend; `tauri.conf.json` wired to the dev server and the build
  output.
- `make dev`, `make build`. No CI yet.
- One `#[tauri::command]` round trip proving the bridge works end to end.

**Done when:** `make dev` opens a window, and a value crosses the IPC bridge in both directions.

**Measure before moving on:** cold start to visible window, recorded as the baseline for N-2. This
number only gets worse from here, so it is worth knowing what it was when the app did nothing.

---

## 2 — Document core (Rust)

**Goal.** Everything that touches a user's file on disk, fully tested, before any UI can call it.
This is the single most dangerous module in the product.

- `document.rs`: `read(path) -> (String, Hash)`; content hashing; the `path → last_known_hash`
  map behind a mutex.
- Atomic write: temp file in the **same directory** (`.medd-<name>.tmp`), `fsync`, copy the
  original's permissions, `rename()` over the target.
- Compare-and-swap: `write(path, content, expected_hash)` re-reads and re-hashes first, rejects
  with `Conflict { current_content, hash }` on mismatch, and records the new hash **before
  releasing the lock**.
- `error.rs`: one error type, serialisable to the frontend.

**Tests (all required before this increment closes):**
- Crash between temp-write and rename leaves the original byte-identical.
- File permissions survive the rename.
- A write with a stale `expected_hash` is rejected and modifies nothing on disk.
- A write records its own hash such that a subsequent identical read is recognised as self.
- Non-UTF8 content, an empty file, a file without a trailing newline, and a symlinked path all
  behave sanely.

**Done when:** no UI exists, and the test suite covers every branch above.

---

## 3 — Workspace and tree

**Goal.** Open a folder, see it, click a file, see its raw text.

- `workspace.rs`: set the root, canonicalise, classify a path as root-relative / loose /
  directory.
- `dir_list(path)` returns **one level**, lazily. Do not walk the tree eagerly — a ten-thousand
  file workspace must not be enumerated before the window appears.
- `dir_list` refuses paths outside the open workspace root. The tree has no business browsing
  elsewhere, and D-15's loose documents explicitly leave the tree unchanged, so nothing legitimate
  needs it. It does not close the arbitrary-read surface described in increment 5 — `document_read`
  cannot be gated without breaking D-15 — but it does mean a caller must already know a path
  rather than being able to enumerate its way to one.
- Sidebar: tree UI, expand/collapse a directory, collapse the whole sidebar (W-3).
- `.md` files are openable; other files are visible and inert (W-8).
- **Dotfiles and `.git/` are hidden from the tree.** A Markdown workspace is nearly always a
  project directory, and repository plumbing in the sidebar is noise while browsing documents —
  which is how comparable tools (Obsidian, Typora, iA Writer) behave for the same reason. Not
  configurable in v0.1; it becomes a setting when `settings.json` arrives in increment 11.
- Clicking a file calls `document_read` and dumps the text into a `<pre>`. **No editor yet** —
  this increment is about the tree.

**Done when:** a folder opens, the tree renders lazily, and clicking a `.md` file shows its
contents.

---

## 4 — Source pane (CodeMirror 6)

**Goal.** Replace the `<pre>` with a real editor. This is the increment with known ramp-up cost
([ADR-002](adr/002-editor-component.md)) and it is placed here deliberately, before anything
depends on it.

- CodeMirror 6 mounted, `@codemirror/lang-markdown`, a theme that reads in light and dark.
- `@codemirror/commands` for undo/redo; `@codemirror/search` for find & replace (Cmd+F) — E-6.
- macOS keybinding fidelity: `defaultKeymap` plus targeted overrides. Verify against real macOS
  editor behaviour, not against browser assumptions.
- The seam that matters: the rest of the app reads **current text** and a **change signal** from
  this component. CM6 types (`Transaction`, `EditorState`, `ViewPlugin`) do not leak into tab
  state, autosave, or the render pipeline. This is ordinary layering, not swappability insurance
  (D-12).

**Smoke-test early, in the real Tauri WebView, not a browser tab:** clipboard copy/paste, IME
composition, and Cmd-key handling. WKWebView has its own history here and it is cheaper to find
out now.

**Budget real learning time for CM6's extension model.** Treating it as a drop-in widget is the
failure mode this increment is sized to avoid.

---

## 5 — Render pipeline

**Goal.** Split view. Source on the left, live preview on the right.

- markdown-it with the GFM plugin set: tables, task lists, strikethrough, footnotes (R-1, R-5).
  Sanity-check the maintenance status of each specific plugin as it is added.
- highlight.js, configured to a **curated language subset** — not every grammar. The subset is a
  named constant; start with bash, rust, python, javascript, typescript, json, yaml, toml, sql,
  markdown, and grow it on demand.
- DOMPurify immediately before insertion into the DOM.
- **Link classification** as a markdown-it renderer rule, not a click interceptor — so the
  classification is visible in the DOM and testable without a browser:
  `http(s)` → `open_external`; relative `.md` → internal link carrying a resolved absolute path;
  in-document anchor → scroll. (R-2, R-6)
- **Image resolution** against the *document's own directory*, not the workspace root — this is
  what makes loose files (D-15) render correctly. Rewritten `src` uses Tauri's asset protocol,
  scoped to the workspace root plus the directories of open loose documents.
- Re-render on a debounce, not per keystroke.
- **Set a real CSP. This is a load-bearing security control, not hygiene.** The skeleton left
  `"csp": null`, which is the scaffolder's default and fine for an app that renders nothing. It
  stops being fine here, and the reason is sharper than "user HTML reaches the DOM":

  `document_read` accepts an arbitrary absolute path **by design** — D-15 says a document outside
  the workspace opens as a loose tab, so there is no path restriction that could be applied
  without breaking a product decision. The command surface therefore legitimately exposes reading
  any file the user can read. That is fine as long as only medd's own code can call it, which
  means the WebView must never execute script it did not ship, and must never be able to make an
  outbound request. DOMPurify decides what HTML survives; CSP decides what the page may do if
  something slips past it, including whether a rendered `<img src="http://…">` can carry data off
  the machine. Neither layer substitutes for the other, and N-6's "works fully offline" is a
  requirement about capability, not just convenience — a page that cannot reach the network cannot
  exfiltrate what it reads.

  Scope the asset protocol to the workspace root and the directories of open loose documents,
  nothing wider.

**Tests — golden files.** A corpus of `.md` inputs with expected HTML fragments covering tables,
task lists, footnotes, strikethrough, fenced code, images, nested emphasis, and each of the four
link kinds. This is the regression net for R-1…R-7 and the thing that makes ADR-001's named
escape hatch (comrak) a measurable change rather than a leap of faith.

---

## 6 — Typography and reading mode

**Goal.** Make it worth reading in. This is product work, not polish — D-3's reading mode exists
because reading is the dominant use, and R-7 asks for a published page rather than an HTML dump.

- One preview stylesheet, shared by split view and reading mode: vertical rhythm, heading scale,
  table borders, blockquote treatment, code-block treatment, list spacing.
- **Measure is the one mode-conditional rule, deliberately.** Reading mode caps at a centred
  ~70ch column, which is where R-7's "published page" is actually cashed in. Split view's preview
  fills its pane uncapped: most split panes are already narrower than that cap, and on a wide
  window imposing it would leave dead margins in a pane the user chose to *share* with an editor
  rather than dedicate to reading. Keep the exception explicit in a comment — read cold it looks
  like an inconsistency to tidy away, and tidying it away would make both modes worse.
- Light and dark, both designed rather than inverted.
- Reading mode: editor hidden, document full-width, sidebar collapsible away to nothing (E-3).
- Mode toggle: split / reading / source-only, per tab (E-4).

**Done when:** a long document with tables and code is genuinely pleasant to read in both themes.
This one is judged by eye, and that is correct.

---

## 7 — Autosave and external change

> **Ordering note.** This increment runs *after* increment 8, not before it. Autosave, derived
> dirty state and the D-11 conflict machinery are all per-document; building them against a
> single-document assumption would mean restructuring the most dangerous code in the frontend
> after it had been tested, which is how bugs get into code everyone believes is covered. The
> original ordering was right that dangerous code should land early and land tested, and wrong
> about how to achieve it. Tabs first, then this written once in the shape it will ship in.
>
> **Gate: withdrawn, and the reasoning corrected.** This increment was briefly gated on a manual
> WKWebView input pass, on the argument that an input bug here would be data loss rather than
> cosmetic. That was over-cautious and the argument does not hold: an input bug produces
> *visibly* wrong text in the editor, which autosave then writes — bad, but not silent. The
> genuinely silent data-loss risks in this increment are the compare-and-swap, the watcher, and
> own-write suppression, all of which are Rust-side and fully testable. The manual input pass
> still matters and stays in increment 12, where it always belonged.

**Goal.** The second dangerous increment. Ship it with the same discipline as increment 2.

- Debounced autosave (~1s after typing stops), calling `document_write` with `expected_hash`.
- Derived dirty state: `dirty === (currentText !== lastSyncedText)`. No stored flag.
- `watcher.rs`: `notify` on the FSEvents backend. Recursive watch on the workspace root, plus the
  parent directory of each open loose document. Coalesce raw events over ~100ms. Filter out
  `.git/`, `node_modules/`, dotfile directories, and anything outside a watched scope.
- Own-write suppression by content hash — the common case, and it must be cheap.
- A genuine external change emits `document:changed-on-disk { path, content, hash }` with the
  content attached, so the frontend needs no follow-up round trip.
- Deletion emits `document:removed-on-disk`; the tab stays open holding its text, marked detached,
  autosave suspended. Never silently recreate a file the user deleted.
- The D-11 banner: clean → silent reload preserving cursor and scroll; dirty → suspend autosave,
  show the banner, offer Reload / Keep mine. `Keep mine` adopts the new hash as the CAS baseline
  and resumes — it does **not** write immediately.
- The watcher runs on its own thread feeding an mpsc channel, so event storms cannot block command
  handling.

**Tests:**
- Rust: a write followed by its own watcher event produces no notification. A foreign write
  produces exactly one, with correct content.
- Frontend, under fake timers: debounce firing; clean reload; dirty banner; Reload; Keep mine; a
  rejected CAS write becoming a conflict. Exhaustive — this state machine is small, stateful, and
  the most dangerous code in the frontend.

**`tree:changed` is emitted but not yet wired to the sidebar** — live tree updates are v0.3 (W-6).
The watcher exists in v0.1 anyway because P-3 needs it.

---

## 8 — Tabs

**Goal.** Multiple open documents (W-4), within the memory budget.

- Tab model: open, close, switch, reorder not required in v0.1.
- **Only the active tab has a mounted `EditorView` and a rendered preview DOM.** Inactive tabs
  retain their CM6 `EditorState` — text plus undo history, which is cheap — and nothing else.
  This is what bounds memory at O(total text) rather than O(tabs × editor machinery), and it
  preserves undo history across tab switches.
- Closing a tab frees everything. No buffer cache, no recently-closed retention.
- Closing a tab is always safe and never prompts (P-2).
- Loose tabs (D-15) show enough of their path to be distinguishable from workspace files.

---

## 9 — Quick-open

**Goal.** Cmd+P (W-5) — the original "button to find and click a file" requirement.

- Fuzzy filename match over the workspace, Enter to open, arrows to move, Esc to dismiss.
- The file list is built lazily and cached; a workspace scan must not block the dialog opening.
- Scoring can be simple. This increment is small and should stay small.

---

## 10 — CLI and single instance

**Goal.** `medd <file>`, `medd <folder>`, bare `medd`, converging on one process (L-2, I-1).
Deliberately late: the fiddliest platform work, and nothing above depends on it.

- `tauri-plugin-single-instance`, socket path derived from a **fixed, stable location** under the
  application support directory — computed identically by the app and the shim, never from the
  invoking binary's own location. This is the invariant.
- `routing.rs`: `route_open(paths)` — canonicalise, classify, dispatch. Handles both listeners.
- `RunEvent::Opened` wired now even though Finder registration is v0.2, so adding the file
  association later is a manifest change rather than an architectural one.
- **The pending-open buffer.** `RunEvent::Opened` can fire before the WebView has attached its
  listeners. Opens are buffered in Rust state and drained by `frontend_ready()`. A cold launch
  goes through this buffer every time — it is the normal path, not the exception.
- Startup race: retry-with-backoff (three attempts over ~100ms) before falling back to becoming
  primary.
- Window activation: attempt `show()` + `set_focus()`, and **depend on neither**. See the known
  limitation below.
- `scripts/medd` shim: if the socket accepts a connection, exec the bundle binary so the plugin's
  own client forwards `argv`; otherwise `open -a` so Launch Services starts it detached from the
  terminal. Resolve relative paths to absolute before handing them on — the running instance's
  working directory is not the user's.
- `make install-cli` symlinks the shim. Homebrew is deferred.

**Known limitation, by design.** `set_focus()` is unreliable on macOS —
`NSRunningApplication.activateWithOptions` has been flaky since Big Sur and is deprecated as of
Sonoma, with two open upstream Tauri issues tracing to it (one closed *not planned*). No IPC
choice fixes this. medd's posture: **the file opens as a tab whether or not the window comes
forward.** Nothing may assume the window is frontmost after a routed open. Spot-test early against
real window states — the reported failure is specific to `hide()`-style states, and medd may never
enter one.

---

## 11 — Welcome state and recents

**Goal.** D-14, and bare `medd` doing something sensible.

- `state.rs`: `~/Library/Application Support/medd/`, via Tauri's path API.
  `settings.json` (the user's, human-editable, written only on change) and `state.json` (the
  app's, ~2s debounce and on quit) kept deliberately separate.
- Both written through the atomic path from increment 2. A crash during a state save must not
  produce a truncated file that stops the app launching.
- **Corruption is not fatal**: a file that fails to parse is renamed `.bak`, defaults are used, the
  app starts. An editor that refuses to launch because its session file is malformed is the worse
  failure.
- v0.1 persists the last workspace and the recent list (capped at 10) only. Full session restore
  is v0.2.
- Welcome pane: app name, *Open Folder…*, recents. Cmd+Shift+O opens the native picker.

---

## 12 — Harden

**Goal.** Replace the estimates with measurements, and do the manual pass.

- **Large-document thresholds.** Benchmark markdown-it re-parse cost against real files and
  replace the estimated 1 MB / 10 MB constants with measured ones. Implement the degradation:
  manual-refresh preview in the middle band, read-only source-only above the top band, each with
  a banner explaining why.
- **Memory soak.** Ten tabs, eight hours. The criterion is resident memory at 8h no more than 10%
  above resident memory at 5 minutes. If it fails, find the retention before shipping.
- **Cold start** re-measured against the increment-1 baseline; confirm N-2.
- **Manual pass:** window focus on second launch (checked, not asserted); WKWebView clipboard and
  IME; macOS keybindings; reading typography in both themes.
- **Restore debug symbols for release diagnosis, or decide not to.** The skeleton set
  `strip = true` in the release profile, which makes a panic backtrace useless. That is the right
  setting for a shipped binary and the wrong one for a product still being hardened; revisit it
  here rather than discovering it while reading an unreadable crash.
- Update `docs/` with anything the implementation taught us that the design got wrong. The design
  documents are not sacred — they are the current best understanding, and increment 12 is when
  that understanding is most improved.

**Definition of done for v0.1:** open a folder of Markdown files, click through them, edit one,
watch the preview update, and trust the change is on disk — without touching the terminal after
launch.

---

## Not in v0.1

Finder association; the Neovim `:Medd` command; full session restore; preferences UI; scroll sync
between panes; table editing helpers; live tree updates; end-to-end driving of the built app.
See [scope-mvp.md](scope-mvp.md) for where each lands.
