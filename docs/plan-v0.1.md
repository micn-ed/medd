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

Each increment is committed locally, verified independently, then pushed. The working conventions
that make that safe — separating authorship from acceptance, stating a fix's invariant before
writing it, and sharing a checkout without losing each other's work — live in
[conventions.md](conventions.md).

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
- **Cycle protection is by resolved-path identity, not a depth cap.** A directory symlink to an
  ancestor — `notes/loop -> ../` — makes the walk unbounded, re-enumerating the same documents at
  every level. `dir_list` never had to care because it descends one level, and `fs::metadata()`
  follows links deliberately here (the dangling-symlink test depends on it). The failure mode is
  the bad one: no crash, no error, no result, a threadpool worker spinning forever while quick-open
  never populates — indistinguishable from a slow walk, on the one operation whose whole promise is
  that it does not block. **A depth cap converts an infinite walk into a silently wrong one**,
  which is worse: it stops, omits everything past the cap, and looks like a correct result.
- **The walk copies the workspace root out of the lock and releases it before touching the
  filesystem.** `commands.rs` holds `Mutex<Option<Workspace>>` across `dir_list` today, which is
  safe *only* because that call returns in microseconds. An async walk holding the same lock for
  its duration would block every sync command — and sync commands run on the main thread, so the UI
  would freeze for the length of the walk. Exactly the outcome the async command exists to avoid,
  reached by holding a lock the old code could hold safely. Testable directly: take the lock on
  another thread mid-walk and confirm it is available.
- **"Must not block the dialog opening" is two claims and needs both.** As written it has no
  threshold and no observable, so it cannot fail — the same shape as increment 10's E2E deliverable
  before its timing constraint was stated. The testable half: the dialog renders and accepts
  keystrokes **while the walk promise is still pending** — assert against an *unresolved* promise,
  since a test that awaits the walk first proves nothing about ordering. The measured half: a
  number on a real workspace, which belongs on increment 12's list beside the large-document
  thresholds. Whether the command carries `async` is a macro attribute and no unit test can see it.
- **The fixture must contain what the filters exclude.** A `node_modules` exclusion test passes
  trivially against a fixture with no `node_modules` — this project has already shipped that
  mistake once, in a harness fixture claiming R-1…R-7 coverage with no images in it.
- **Cache invalidation needs the half that establishes a stale result would otherwise be served**,
  or the other half proves nothing. And invalidation **marks stale rather than re-walking**, or a
  `cargo build` becomes a sequence of full workspace walks.
- **Every ignore rule carries a mutant** in `scripts/mutants.sh`. A rule with no mutant is a rule
  nothing is checking, and the harness is where that stays visible rather than depending on anyone
  remembering.
- Scoring can be simple. This increment is small and should stay small.

---

## 10 — CLI and single instance

**Goal.** `medd <file>`, `medd <folder>`, bare `medd`, converging on one process (L-2, I-1).
Deliberately late: the fiddliest platform work, and nothing above depends on it.

- `tauri-plugin-single-instance`. **The socket path is the plugin's, not medd's** — it hardcodes
  `/tmp/<identifier>_si.sock` with no configuration hook, so it derives from `config.identifier`,
  a compile-time constant. Do not compute a path anywhere: the invariant that matters (it depends
  on nothing about the invoking binary's location, so the shim and the bundle never need to agree
  on where the other lives) is already satisfied by the plugin. An earlier version of this plan
  named a location under Application Support, which nothing ever binds.
- `routing.rs`: `route_open(paths)` — canonicalise, classify, dispatch. **Routing and activation
  are separate**: `route_open` handles the two listeners that carry documents, and `activate()` is
  called by all three. `RunEvent::Reopen` carries no paths and never enters the router.
- `RunEvent::Opened` wired now even though Finder registration is v0.2, so adding the file
  association later is a manifest change rather than an architectural one.
- **There are four entry points carrying paths, not three, and the fourth is deferred.**
  `WindowEvent::DragDrop` delivers `Vec<PathBuf>` from the webview layer — neither `argv` nor an
  Apple Event — so it bypasses both listeners below. It is enabled by default and unhandled, so
  dropping a file on medd's window currently does nothing. **Scoped to v0.2** with Finder
  registration (see scope-mvp.md), because it is the same family and splitting them would be
  incoherent. Recorded here so the next person enumerating finds four and a note rather than three
  and a gap. When it lands it is one more listener calling `route_open` — and it is the one
  listener that never needs the pending-open buffer, since a drop requires a live window and a
  ready frontend. That must be **deliberate rather than incidental**: it shares the code path that
  buffers.

  `tao` registers exactly seven `NSApplicationDelegate` methods, which makes this enumeration
  closed rather than merely long — anything absent cannot reach medd whatever `Info.plist` says.
  Handoff and inbound Services are registered but inert (`NSUserActivityTypes` and `NSServices` are
  undeclared), and `applicationShouldTerminate:` is registered by nothing, which is the Cmd+Q
  finding already in hand.
- **Verification is per entry point, not per function.** Both bugs found in this subsystem were in
  the *wiring*, not the router: Cmd+Q never reached the hook, and window-close reached it too late.
  In both cases the thing being called was correct and a test of it would have passed. So a green
  `route_open` suite is exactly what *covering one is indistinguishable from covering all* looks
  like — it tests the one part of this subsystem that has never been broken.

  **The criterion is two claims, not one, and an earlier version of this section conflated them.**

  **(a) Per-listener *shaping* is unit-verified.** Each listener's path extraction is a named,
  Tauri-free function — `paths_from_argv(&[String], &Path) -> Vec<PathBuf>`,
  `paths_from_urls(&[Url]) -> Vec<PathBuf>` — with the closure reduced to one line calling it.
  Then: **break only that listener's extraction and confirm exactly one test fails, named for that
  listener.** This catches the `argv[0]` bug, the missing `file:` filter, and relative-path
  resolution, each distinguishably. It requires the extraction to exist, or the criterion has
  nothing to bite on.

  **(b) Per-listener *hook choice* is gesture-verified, and one gesture is deferred.** Which
  platform event a listener registers for cannot be reached by any unit test. `MockRuntime::run`
  emits only `Ready`, `WindowEvent{CloseRequested}`, `ExitRequested`, `MainEventsCleared` and
  `Exit` — never `Opened` or `Reopen`, which are macOS delegate-driven — and the single-instance
  callback is invoked by the plugin's socket listener rather than the runtime. So hook choice needs
  a real gesture per listener. **`RunEvent::Opened`'s hook correctness is unverifiable in v0.1**,
  because it cannot fire without `CFBundleDocumentTypes`, which is v0.2. State that rather than
  leave it implied: the handler is written, its shaping is tested, and whether the event ever
  arrives is a v0.2 question.

  **Why the split matters more than being precise.** The earlier single criterion could not catch
  either of the two bugs it was written for — both were about *which hook was chosen*, which lives
  in registration code no test reaches, while the criterion covers route *shaping*. A criterion
  that appears to guard the thing that keeps breaking is worse than none, because it **retires the
  question**: the same failure as `own_write_produces_no_notification`, which asserted something
  true and adjacent while the behaviour it was named for was broken.

  The `route_open` unit tests are still worth having; they are simply not evidence about hook
  choice.
- **`route_open` takes owned paths and returns a decision for the caller to act on.** "It must not
  block" has no observable and no threshold, so as a requirement it cannot fail — the same shape as
  §9's dialog bullet before it was split. But unlike that one it has a structural answer, because
  it is a constraint on what the code may *contain* rather than on what it computes: a function
  given only owned paths, returning a decision, **cannot reach a dialog or a walk**. The constraint
  then holds by construction instead of by vigilance. Same move as the lock extraction's owned
  root — and the third time on this project that an unfalsifiable property has turned out to be
  expressible in a signature.
- **The pending-open buffer's test must emit *before* the frontend is ready.** This is the easiest
  thing in the increment to test vacuously, and it is also the most load-bearing, since §10 calls
  it the normal path for every cold launch. The natural test — attach listeners, emit an open,
  assert the tab opened — **passes with no buffer whatsoever**, because the listener was already
  there. It reads as "opening a file works" and exercises none of the buffering. The discriminating
  order is the inconvenient one: emit, *then* attach, *then* drain. The awkward sequence is the
  real one, which is exactly why the comfortable one gets written.
- **Draining needs both halves asserted.** A test checking only that nothing is *lost* passes
  against an implementation that never clears the buffer and re-opens every file on each WebView
  reload. "Nothing lost" and "nothing duplicated" are separate claims and only the first is
  obvious.
- **"Never start a second process" is not unit-testable, and the parts that are must not be
  mistaken for the whole.** The shim's argv handling is shell-testable, the refused socket bind is
  Rust-testable, the forwarded paths are frontend-testable — but *convergence* needs two real
  processes and nothing smaller. Decompose it explicitly, as the quit gesture was, so the covered
  fraction is visible.
- **Relative path resolution must be tested through the shim.** Every existing path test passes
  absolute paths, because `canonicalize()` requires the target to exist — so none of them exercise
  resolution at all. And since the shim is authoritative for it, a single-process test
  *structurally cannot* fail the claim, which is about two processes disagreeing about `cwd`.
- **The pending-open buffer.** `RunEvent::Opened` can fire before the WebView has attached its
  listeners. Opens are buffered in Rust state and drained by `frontend_ready()`. A cold launch
  goes through this buffer every time — it is the normal path, not the exception.
- Startup race: **not mitigated** — see ADR-003. The plugin unlinks then binds, so two simultaneous
  launches produce two windows and there is no hook to retry from. Accepted and documented.
- Window activation: `activate()` is **`unminimize()` → `show()` → `set_focus()`**, in that order.
  `unminimize()` is not optional — `tao`'s `set_focus()` guards itself behind `isMiniaturized()`
  and `isVisible()`, so without it activation on a minimised window is skipped entirely *and
  returns success*. Query the window's own `is_minimized()`/`is_visible()`; do **not** branch on
  `Reopen`'s `has_visible_windows`, which the CLI path never receives.
- Activation is verified in **three named states with predicted outcomes** — occluded, minimised,
  hidden — using `is_focused()` to assert rather than eyeball. Only the occluded case is expected
  to remain unreliable. See the known limitation below.
- `scripts/medd` shim: **one path, no probe.** Always exec the bundle binary
  (`medd.app/Contents/MacOS/medd`) with the user's arguments and let the plugin make the warm/cold
  decision itself — it already does so correctly, atomically and exactly once. Do not probe the
  socket first: a bare connect-and-close makes the primary read zero bytes, which becomes an empty
  argument list, **which fires the primary's open callback**.
- **Detachment is not optional.** Launch with `nohup … &` or equivalent. Measured on a real bundle:
  executing the binary directly gives correct bundle identity and `type="Foreground"`, and with
  detachment the process reparents to `launchd` and outlives its terminal — without it, the
  cold-launch case dies with the shell, which is the failure `open -a` originally prevented.
  Dropping `open -a` silently drops that property unless it is deliberately restored.
- Resolve relative paths to absolute before handing them on — the running instance's working
  directory is not the user's.
- **Named deliverable: the end-to-end quit gesture.** The quit flush is verified at the unit level
  and by a real Cmd+Q against an app with no dirty buffer; the path that matters — *dirty buffer,
  real keystroke, correct bytes on disk* — is inference until this increment. It is blocked today
  only on opening a document without clicking, because WKWebView content is not reachable through
  the accessibility tree. The CLI removes that block: `Editor.svelte` calls `view.focus()` on
  mount, so a document opened via `medd fixture.md` leaves the editor focused and every step after
  is keyboard-only.

  ```
  medd fixture.md          # cold launch -> pending-open buffer -> openTab -> focus
  keystroke "x"            # proven
  keystroke cmd+q          # WITHIN the debounce window — see below
  read fixture.md          # assert exact bytes
  ```

  **The timing is the whole test, and stating it is not optional.** `AUTOSAVE_DEBOUNCE_MS` is
  1000, so an ordinary autosave writes `x` to disk one second after the keystroke with no quit
  involved. Without a timing constraint this script **passes against a quit flush that is broken,
  silently does nothing, or has been deleted outright** — at any human pace, any `osascript` round
  trip, or any `sleep 1` someone adds to "let it settle". The named deliverable for the one path
  that is otherwise inference would be vacuous by default, and vacuous in the direction that reads
  as success.

  Four requirements, all load-bearing:

  - **The gesture lands inside the debounce window** — under ~1s from the last keystroke. Only then
    is the flush under test rather than the ordinary autosave.
  - **Assert the timing rather than assuming it.** Measure keystroke→gesture elapsed and fail the
    run if it exceeds the window. A slow machine or a cold `osascript` otherwise converts this
    silently into the vacuous version, which stays green, so nobody looks. **The test must be able
    to detect that it has stopped testing anything.**
  - **Run the negative control**: the same script, same timing, against a build with
    `app:before-quit` disabled. It must fail. A script never observed failing has an unknown
    failure mode.
  - **Assert exact bytes, not a substring.** `contains "x"` also passes on a truncated or
    duplicated document.

  **Run it from a cold launch**, not against a warm instance: keystrokes need the window frontmost,
  activation is best-effort, and a cold launch *is* frontmost. Note `openFile` is latched during
  shutdown, so the open must complete before the quit gesture — which it naturally does.

  **Two more scenarios become keyboard-reachable for the first time here**, and are cheap to take
  while the harness exists: **quit with several dirty tabs** — unit-verified but never exercised
  against real files, and multi-tab scale is what made this a blocker rather than a repeat of the
  single-tab case — and **quit with a conflicted tab**, where the file must still hold the external
  change afterwards.
- `medd <nonexistent-file>` **errors**; it does not create. See architecture.md §9 for why, and
  note the error must not use a surface that replaces the tab UI.
- `make install-cli` symlinks the shim. Homebrew is deferred.

**Known limitation, by design — and only in one of three states.** An earlier version of this
paragraph called `set_focus()` uniformly unreliable and said the failure was specific to
`hide()`-style states "which medd may never enter". Both halves were wrong, and the framing would
have let a deterministic skip ship as an accepted platform cost.

- **Occluded** — genuinely unreliable. `activateIgnoringOtherApps:` has been flaky since Big Sur
  and is deprecated as of Sonoma; two upstream Tauri issues trace to it, one closed *not planned*.
  No IPC choice fixes this, and this is the real limitation.
- **Minimised** — *fixable*, and fixed by `unminimize()`. `tao` guards `set_focus()` behind
  `isMiniaturized()`, so without it the call is skipped entirely and returns success.
- **Hidden** — probably fixable by `show()`; to be measured. Cmd+H is `[NSApp hide:]`, which hides
  the *application*, and a window-level call may not clear an application-level flag. If `show()`
  is insufficient the choice is an `objc2` call to `unhide:` or accepting it — decided against a
  measurement, not in advance.

medd never enters a hidden state only if medd never ships the menu item — and it does: Tauri's
`Menu::default` puts **Cmd+H** and **Cmd+M** under medd's own application menu.

For the occluded case the posture stands: **the file opens as a tab whether or not the window comes
forward**, and nothing may assume the window is frontmost after a routed open.

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

## Carried fixes — from the increment 7 design review

Found by the principal architect reviewing increment 7 against the design
([review-increment-7.md](review-increment-7.md)), verified by running code. Two blockers were
fixed inside increment 7. These five are real, are not silent data loss on a path the design calls
safe, and are therefore queued rather than blocking — but none of them ship broken.

| # | Finding | Why it matters |
|---|---|---|
| 3 | A write settling after a conflict resolution clobbers the resolved state | Produces a conflict banner that no user edit caused — D-11's own named fatal failure. Needs a per-tab generation counter captured before the await. |
| 4 | `detached` is a terminal, invisible state | A deleted file silently stops autosaving forever, with nothing on screen and no recovery — a recreated file comes back untracked. `git checkout` across branches does exactly this. |
| 5 | Any read failure is reported as deletion | `EACCES`/`EIO`/`EMFILE` — the last most likely during the filesystem storms that generate watcher traffic — all latch a tab into finding 4's state. Only `NotFound` should mean removed. |
| 6 | A non-UTF-8 external change is lossily converted, and the CAS lets medd write it back | `read()` refuses non-UTF-8 but `check_external_change` uses `from_utf8_lossy`, and the hash is of the raw bytes — so a later autosave passes CAS and writes replacement characters over the file's real content. **The line-ending fix now inherits this exposure:** detection in `read()` only ever sees valid UTF-8, but in `check_external_change` it runs on lossy output. A UTF-16LE document — an ordinary way for a `.md` to arrive from Windows — decodes to `\r\0\n\0` per break, so no `\r\n` is found, the lone-`\r` and lone-`\n` counts tie, a CRLF file is detected as LF, and the next write rewrites every line ending. 6's ruled fix closes this completely, since the content never reaches detection. |
| — | `document_close` is specified in architecture.md §4 and does not exist | `last_known` grows for the session, loose-document watches are never released, and asset-protocol grants are never revoked. |

**Cmd+W quits medd, and quitting loses every pending edit.** Sequenced after the current fix
batch, not into it, but ranked with the blockers rather than below them.

Tauri's `Menu::default` ships Cmd+W twice, and under I-2 it quits. In every other tabbed editor on
the platform Cmd+W means *close tab*, so a user with six documents open reaches to close one and
quits the app. Meanwhile `main.rs` registers no `ExitRequested` handling and the frontend has no
unload path, so any edit inside the ~1s debounce window — **in every open tab** — is lost on quit.
That is QA's finding 1 generalised from one tab to the whole app, behind a keystroke that means
"close one tab" to the user's fingers. P-2 promises closing a tab is safe; nothing promises
quitting is, and under D-5 no user has reason to draw that line.

**The two halves are coupled for correctness, not merely sequenced for UX — do not split them.**
`RunEvent::ExitRequested` is produced from exactly two places in `tauri-runtime-wry`: the last
window being destroyed, and a programmatic `AppHandle::exit()`. **Cmd+Q reaches neither.** It is
`[NSApp terminate:]`, whose only cancellation point is `applicationShouldTerminate:`, which `tao`
does not implement — so it goes straight to `LoopDestroyed` → `RunEvent::Exit`, unpreventable and
already committed. `Exit` is unusable for a flush in any case: it arrives inside a delegate
callback, `tao` drops the event-loop callback immediately after, and the WebView cannot be pumped
from there. Since the frontend owns the document text, **Rust cannot flush on that path even in
principle.**

A flush built on `ExitRequested` alone would cover window-close and programmatic exit, miss the
most common quit gesture on macOS entirely, and pass every test written against `ExitRequested`.

The fix is nearly free and exists *only* because the Cmd+W remap already replaces `Menu::default`:
build Quit as a **custom** item with a `Cmd+Q` accelerator whose handler calls `app.exit(0)`,
routing through `RequestExit` → `ExitRequested` → preventable → flush → real exit. Deferring the
menu work would therefore silently stop the flush covering its own main path.

`terminate:` still arrives from the Dock's Quit, the force-quit dialog, and SIGTERM/SIGKILL, and
nothing can flush those. **That is why the atomic-write guarantee is the real protection and the
flush is an optimisation on top of it** — the flush must never be treated as the thing keeping the
file safe.

Both halves ship together: fixing the keystroke alone hides the data loss behind a rarer gesture,
and fixing the flush alone leaves an editor that quits on Cmd+W.

**Verified before building behind it:** a custom menu item bound to `CmdOrCtrl+Q`, with a handler
calling `app.exit(0)`, does receive its `MenuEvent` normally on this stack (tauri 2.11.5 /
tauri-runtime-wry 2.11.4 / tao 0.35.3) — tested end to end with a synthetic Cmd+Q. The whole design
rests on that one assumption, so it was measured rather than assumed.

Two macOS menu behaviours found while testing it, both of the silently-accepted kind:

- **A top-level `Menu`'s children must be `Submenu`s.** A bare `MenuItem` appended directly to the
  top-level menu is accepted by the API without error and simply never appears in the menu bar.
- **macOS forces the *first* top-level submenu's displayed title to the application's name**,
  whatever string was passed. Not a problem in the real build, where the app-identity submenu is
  first exactly as in `Menu::default` — recorded so it isn't mistaken for a bug the next time
  someone notices it. The keystroke needs a custom menu
replacing `Menu::default` (Quit stays Cmd+Q); the flush needs `ExitRequested` + `prevent_exit()`,
asking the frontend to flush and exiting when it reports done. Its invariant is the close-flush's
with clause 2 vacuous: **the exit must issue everything the debounce still owes, and the outcome of
those writes applies to nothing.** Four more, worked out ahead of the code:

- **`ExitRequested` is the right hook only for exits that do not begin by destroying a window.**
  On the window-close route it fires from `Destroyed` — *after* the window is gone from Tauri's
  store — so an emit to the frontend reaches no webview, silently. Closing the window must be
  intercepted one event earlier, at `RunEvent::WindowEvent { CloseRequested { api } }`, which
  arrives while the frontend is still alive and carries `prevent_close()`. **Both routes reach the
  same `begin_shutdown`**, rather than two handlers that each happen to call the flush.
- **The bounded wait runs on a spawned thread, never in the handler.** Blocking in the handler
  stops the main thread pumping, so the WebView can never deliver its "ready" signal and the JS
  flush never runs — every quit would take the full ceiling and flush nothing, while the wait's own
  unit tests passed.
- **A repeat user request also prevents, and does nothing.** Having it skip the remaining wait was
  considered and rejected: because `flushAll` cancels the debounce, a normal flush is tens of
  milliseconds, so medd has exited before a human can press twice in essentially every real case.
  The skip therefore fires *only* when a write is stalled — the case where the edit is most at risk
  and truncating costs most. It also misreads the gesture: with no feedback of any kind, a second
  press far more likely means "did that register?" than "yes, I mean it", especially in an app
  whose whole save story is that you never think about saving. The ceiling is already the escape
  hatch. Note this needs two conditions kept distinct — the coordinator's own completion must pass
  through where a repeat user press must not, and one branch cannot serve both.
- **The bound is a Rust-side timer that exits regardless of what the frontend says**; the
  frontend's "done" may only make it sooner. If the frontend is the only thing that can end the
  wait, a JS exception or an already-crashed WebView leaves medd *unquittable*, and the user's only
  recourse is force-quit — the one path with no flush at all. Prefer a short bound: an expired
  bound costs a second of typing, an unquittable editor costs everything.
- **The handler is idempotent and a second request must not restart the bound.** With no
  "flushing" UI, a user who presses Cmd+Q and sees nothing presses it again. Better still, a second
  request *skips* the remaining wait — that is what the gesture means, and it is a way out that
  isn't force-quit.
- **Shutdown latches**: once it begins, `scheduleAutosave` is a no-op and `applyExternalContent` is
  suppressed. The watcher is live during the drain, and a `document:changed-on-disk` on a clean tab
  reschedules autosave — so quiescence recedes by a second, repeatedly, and a `git checkout`
  landing during shutdown is enough. Without the latch the bound is terminating a drain bug rather
  than guarding an unlikely one.
- **A quit-time write must be able to be rejected.** A direct write would surrender atomicity, but
  the subtler loss is the compare-and-swap: skipping the hash re-check overwrites an external
  change made while medd sat idle — what D-11 exists to prevent — and at quit there is no chance of
  a banner, so it resolves silently in medd's favour on the one path where the user isn't watching.
  If a CAS fails during the flush, **drop that write and exit**; do not force it through. Losing an
  edit is recoverable because the user still has their file; silently overwriting someone else's
  change is not.

Mechanically these reduce to one rule: every quit-time write goes through `document_write` and
therefore `DocumentStore::write`. No new command taking a list of buffers, no Rust-side shortcut
for shutdown. The close-flush is therefore to be built as a primitive with more
than one caller, since quit is the second.

From QA's systematic mutation sweep over `tabs.svelte.ts` and `doc.ts` — both coverage findings,
not defects; the code is correct in each case:

| # | Finding | Why it matters |
|---|---|---|
| 9 | `does not fire when the buffer is not dirty` is vacuous | `scheduleAutosave` is reachable only through `onDocChanged`, which fires only on an edit — so the test opens a tab, edits nothing, and no autosave is ever scheduled, meaning the dirty guard never runs. Mutation-proven: deleting the guard does not kill it. The test that would exercise it is a real scenario rather than a contrivance — edit, then let a clean external reload land *before* the timer fires, so the tab is clean when `requestWrite` runs. Someone types, a `git checkout` reverts the file to match, and the pending timer must not write. |
| 10 | The `markSynced` invariant is protected only by accident | Mutating `markSynced` to record `tab.currentText` instead of `syncedText` — exactly the bug its own comment warns about — is killed by a *single* test about `waitForQuiescence` chaining, which is about something else entirely. The bug it silently guards: type `A`, the write starts carrying `A`, type `B`, the write settles, `markSynced` records `AB` as what is on disk. `dirty` becomes false, so **`B` is never written and never will be** — a lost keystroke, no banner, no error, in the increment built to prevent exactly that. The invariant survives only as a side effect of a test that could be restructured for unrelated reasons by someone with no idea what went with it. One explicit test, named for the invariant: *a write's outcome records what was written, not what the buffer holds when it settles.* |

From QA's retroactive pass over increments 1-6 and 8:

| # | Finding | Why it matters |
|---|---|---|
| 7 | Every image renders with `alt=""` | `imageResolution` replaces markdown-it's image rule and drops the step that copies inline children into `alt`. More than accessibility: `images.ts` argues a broken-image icon is "the correct, honest signal" for a remote image the CSP blocks — and that argument depends on the alt surviving, because the alt is what tells the reader what didn't load. A documented rationale silently stops being true. One line, before the `src` rewrite. |
| 6 | The find/replace panel is unthemed — a light slab in the dark editor | `EditorView.theme({…})` is called with no second argument, so CodeMirror tags the editor light regardless of the colours it paints; the dark rules ship and never match. Not `{dark: true}` — that is static and medd's scheme is decided at runtime by `prefers-color-scheme`. Drive the panel from the app's own variables, as the rest of the file does. |
| 8 | The harness fixture claims R-1…R-7 and contains no images | R-4 is the missing one, and it is the requirement with the silent-failure history (CSP blocking `data:`; remote images deliberately not rendering). A `data:` image renders for real in the harness and is genuine coverage; a relative one only proves the rewrite fired. Both, labelled for what each proves. |
| — | ~~Interrupted writes leave `.medd-*.tmp` litter~~ **Done.** | Swept after a successful `document_write`, since litter is created by writes and the directories that can hold it are exactly those medd has written to. Abandonment is established by **a file lock the writer holds**, not by an age heuristic — the lock *is* the fact, the kernel releases it on crash, and it handles the two-instance race exactly rather than probabilistically. An earlier version of this row specified sweeping files "older than about a minute"; that design was rejected and this row is corrected rather than deleted, because the superseded version is what someone reading it would have built. |
| — | `--error` keeps its light value under the dark media query | Marginal contrast on a dark canvas. A value to set, not a finding. |

Record now, fix before v0.3: **own writes emit `tree:changed`**. An atomic write reports three
paths to FSEvents — the directory, the document, and `.medd-*.tmp` — and own-write suppression is
specified only in terms of the document's content hash, so two of the three set `tree_changed`.
Harmless until W-6 lands in v0.3, at which point every autosave re-lists every expanded directory.
Note that the existing `own_write_produces_no_notification` test **cannot** catch this: it asserts
per-path classification, never the emitted-event decision, so it passes while the behaviour it is
named for is violated. The deeper cause is that `run_event_loop` is never executed by any test at
all, while holding every observable decision — so this is a symptom, not a separate defect. The
ruling is to lift the decision out as `decide(events, root, &DocumentStore) -> Vec<WatcherEvent>`
with `run_event_loop` reduced to a thin emit loop; **batch in, batch out, never per-path**, because
`tree_changed` is a fold across the batch and a per-path signature pushes that fold straight back
into the untestable shell. Keep the real store and the real filesystem — the existing test already
had the evidence in `paths_to_check`'s output and simply asserted one layer too low; an injected
classifier would only assert against paths the author thought to enumerate, and nobody enumerates
the temp file. Acceptance criterion: one real `document_write`, through a real watcher, against a
real `DocumentStore`, asserting the complete emitted event set is empty.

A second bug lives under the same lens: **a tracked document's deletion emits
`document:removed-on-disk` and no `tree:changed`**, so the tree never learns the file is gone. An
*untracked* file's deletion does emit one, which means the only deletions invisible to the tree are
of currently-open documents — the worst subset.

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
- **The quit ceiling is a budget with a named worst case, and belongs on this list.** 3s is
  ~3× a pessimistic ten-tab flush, where the cost is **N serialised `fsync`s** — the writes contend
  on `DocumentStore`'s single mutex — under concurrent load from `git checkout` or Spotlight. It is
  *not* the autosave debounce: `flushAll` cancels every pending timer and issues immediately, which
  is its whole purpose. Measure it the way the large-document thresholds are measured.

  Keep it **constant, not adaptive**. Scaling with dirty-tab count is backwards: the ceiling bounds
  the *pathological* case, and a stalled write does not scale with tab count — an adaptive bound
  would grant the pathological case more time precisely when more tabs are open.

  Recorded but not built: the ceiling bounds *total work* while the hazard is *stall*. A
  progress-reset rule — restart the clock on each completed write — bounds the right thing: ten
  healthy writes take as long as they need, one stalled write still times out. §8 sets no cap on
  open tab count, so enough dirty tabs to hit a flat bound with nothing actually wrong is reachable
  in principle. Do this if the soak test or the no-cap decision makes the flat bound bite.
- **Window-close ordering is verified by reading `tauri-runtime-wry`, and re-verified by reading
  it again after any Tauri upgrade.** Worse than a test, and the only thing available:
  `MockRuntime` cannot drive medd's exit handling at all. Its run loop breaks on exactly one
  condition — the window map emptying with the resulting `ExitRequested` *not* prevented — medd's
  handler prevents on precisely that path, and the mock's message enum has no `RequestExit`, so
  `AppHandle::exit(0)`, medd's escape from its own prevented exit, does nothing under it. Driving
  the real exit handling under the mock **hangs by construction**. See conventions.md on why the
  terminating consolation test was declined.
- **One drag inside the editor.** `drag_drop_enabled: true` installs Tauri's own drag handler on
  the webview, and Tauri's docs say disabling it is required for HTML5 drag-and-drop on the
  frontend **on Windows** — so macOS is probably unaffected. But CodeMirror uses HTML5
  drag-and-drop to move selected text, which E-6's "standard editing affordances" implies, and
  nobody has looked. Probably nothing; cheap to check; annoying to find after shipping.
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
