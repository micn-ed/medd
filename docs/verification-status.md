# Verification status

**Owner:** QA
**Accurate as of:** `c80b619` — 130 Rust tests, 137 frontend tests, all green; clippy `-D warnings`,
`cargo fmt --check`, `svelte-check` clean.

This document exists because **the test counts above are weaker evidence than they look**, and the
gap is not obvious from reading them. It says what is actually established, what is established
only by reading code, and what no test here can reach. It does not list the fixes themselves —
[plan-v0.1.md](plan-v0.1.md) has those, and they move daily.

If you are new: read §1 and §2 before trusting any green result on this project.

**Every claim here was checked against the tree when written, not recalled.** That mattered: two
of the four items I had listed as open turned out to be fixed — the sidebar-visibility leak, and
the coverage half of the vacuous-test finding. Both had been real, both were reported by me, and
my memory of them was a release out of date. **If you are reading this more than a few days after
the commit named above, re-check before acting** — the checks used are named inline so they can be
re-run rather than re-derived.

---

## 1. The one thing to know first

**A passing suite proves the tests pass. It does not prove they would fail if the code were
wrong.** Those are different claims, and the gap between them has been where nearly every real
defect on this project lived. Three examples, all shipped against green suites:

- A compare-and-swap rejection never reached the user — `{"kind":"conflict","current_content":…}`
  from Rust against `kind === 'Conflict'` and `e.currentContent` in the frontend. **Six** frontend
  tests mocked the shape the frontend wanted. They agreed with each other, and with nothing else.
- Reading mode's measure was specified at `70ch` and rendered at 91–96 characters, because `ch` is
  the width of the "0" glyph, not of an average character. No test asserted a character count.
- The folder picker deadlocked on every click, on every copy, for two days.

The inversion worth internalising: **over a mocked boundary, more passing tests means it is less
likely someone looks, not more likely it is right.** The count measures exposure.

---

## 2. What each instrument can and cannot establish

| instrument | establishes | structurally cannot |
|---|---|---|
| **Rust tests** (130) | filesystem behaviour, the document core, watcher classification, quit coordination — against real temp files | anything above the IPC boundary; anything needing a running app or a window |
| **Frontend tests** (137, vitest/jsdom) | the autosave and conflict state machine, tab model, render pipeline, matching | anything about what Rust actually sends — see the wire pins below; anything about rendering, since jsdom lays nothing out |
| **Browser harness** (`npm run harness`) | typography, layout, measure, themes, tree, tabs, mode toggling — by eye and by measurement | **it is Blink; the app is WKWebView.** Clipboard, IME, native key handling, the real CSP, and the asset protocol are all untouched. It has no backend: `tauriMock.ts` never writes to disk and nothing can emit a Rust-side event |
| **`wire_format` pins** (7, in `src-tauri/src/*.rs`) | that Rust's field and variant *names* match what the frontend reads | **vocabulary, not semantics.** They would not catch a path relative to the wrong root, or a hash of the wrong bytes |
| **`scripts/mutants.sh`** (22 mutants) | that each guarded decision is detected by at least one test | only the decisions someone wrote a mutant for. See §5 |
| **`command_shape` check** | that no `#[tauri::command]` calling a `blocking_*` API is missing `(async)` | one specific deadlock class, by reading source. Nothing else about runtime behaviour |

**The harness deserves its own warning.** A green harness says nothing engine-specific. It exists
because Tauri on macOS has no WebDriver (`tauri-driver` is Linux and Windows only), and it earned
its place — reading mode's measure bug was found in it — but its scope is narrow and easy to
over-read.

---

## 3. Open items that already have a reproduction

**Do not re-derive these.** Each has concrete values and a location.

### 3.1 Reading mode overflows its container by 104px — OPEN

`Preview.svelte:76` sets `height: 100%`; `preview.css:47` adds `padding: 2.5rem 1.5rem 5rem`
(120px vertical); there is no `box-sizing: border-box` anywhere in `src/`. The only `border-box`
rules on the page come from CodeMirror's injected stylesheet.

Measured in the harness at a 1512×725 viewport:

```
.preview.reading   box-sizing : content-box
                   height     : 592.469px      (the 100% resolution)
                   padding    : 40px + 80px
                   used height: 712.47px       <- 592.469 + 120
parent .single-pane client height: 620px
document scrollable by: 104px                  <- exactly header + tab bar + toolbar
```

**Consequence:** in reading mode the whole app document is scrollable by 104px, so any scroll that
lands on the document rather than the preview scrolls the entire chrome off-screen — no header, no
tab strip, no mode toggle, no sidebar, and no visible way back. Split and source modes are
unaffected (`documentScrollableBy: 0`), because `.preview:not(.reading)` has zero vertical padding.

**Easiest reproduction:** open a document with a footnote, switch to Reading, click a footnote
reference. Footnote links come from `markdown-it-footnote`'s own renderer rules, so they bypass
`linkClassification` and carry no `data-link-kind`; `Preview.svelte`'s handler returns before
`preventDefault`, and native fragment navigation scrolls the document.

**Causation proven**, not inferred — toggling the property on the live element:
`content-box → 712.5px / 104px scrollable`, `border-box → 592.5px / 0`, and back.

### 3.2 A detached tab never recovers — OPEN

`markDetached` sets `tab.detached = true` and nothing anywhere sets it back (`tabs.svelte.ts` has
`detached: false` only at construction). Autosave is suspended for that tab permanently.

Three routes out, all closed, each with a characterisation test already in the suite
(`src/doc/detached-recovery.test.ts` — **green, asserting the wrong behaviour deliberately**; they
go red when this is fixed, which is the point):

- the file returning to disk does not clear it — Rust stops tracking the path on removal, so no
  `document:changed-on-disk` is ever emitted for it again
- re-opening from the tree does not clear it — `openTab` early-returns for an already-open path,
  discarding the freshly read content *and* hash
- typing into it is silently never written — 60s of edits, zero `document_write` calls

The exit is closing the tab, which P-2 promises is always safe, and which takes the buffer with it.

### 3.3 A vacuous test remains in `doc.test.ts` — minor, coverage now closed elsewhere

`does not fire when the buffer is not dirty` opens a tab, makes **no edit**, advances timers, and
asserts no write. `scheduleAutosave` is only reachable through `onDocChanged`, which fires only on
an edit — so no autosave is ever scheduled and `requestWrite`'s dirty guard never executes. It
passes because nothing was scheduled, not because the guard works.

**The coverage gap it represented is closed**: `a stale pre-conflict timer, left uncancelled by
Reload, still finds nothing dirty to write` now exercises the guard properly — deleting
`if (tab.currentText === tab.lastSyncedText) return` kills that test. So this is tidying rather
than a hole: a test that establishes nothing still sits in the suite, and its name claims
otherwise.

---

## 4. Items verified by reading only

These are believed correct, and the belief rests on reading the code rather than on observing the
behaviour. Each says what would settle it.

| item | why reading only | what would settle it |
|---|---|---|
| **The folder picker fix** | the deadlock is between a dependency and the platform event loop; no unit test, harness run, or mutant can see it | one click. The `command_shape` check guards the *class* structurally, which is a different guarantee from this instance working |
| **Window-close flush ordering** | `WindowEvent::CloseRequested` fires before the webview is destroyed, `ExitRequested` after — the ordering is a platform property, asserted from documentation and inspection | closing the window with a dirty tab inside the debounce window, and reading the file |
| **The three activation states** | occluded / minimised / hidden behave differently and `set_focus()` is known-unreliable; the design says activation is best-effort | spot-testing each state against a real window |
| **The end-to-end quit gesture** | see §6 | a keyboard-only script, which increment 10's CLI makes possible for the first time |
| **CSP and the asset protocol** | the harness is a Vite dev server with no CSP; the real policy has never been exercised | loading a built `.app` and attempting a blocked resource |
| **Clipboard, IME, macOS keybindings** | WKWebView differs from Blink and the harness cannot speak to it | increment 12's manual pass |

---

## 5. The mutation harness

```sh
scripts/mutants.sh [frontend|rust|all]      # default: all
```

22 mutants. Each breaks one decision and expects the suite to notice. It refuses to run on a dirty
`src/` or `src-tauri/`, because it edits source in place and restores from git.

**What it does not cover:** only decisions someone wrote a mutant for. A new guard with no mutant
is a guard nothing checks — adding one is part of adding the guard.

**Two failure modes that look like success**, both of which have happened here:

1. **A mutation that does not apply, or does not compile.** A malformed mutation injects a syntax
   error, fails every test at once, and reports a *huge* kill — a false result that looks better
   than a real one. The harness now fails loudly on a non-applying mutation and compile-checks
   every mutated tree. This caught a real instance: the field separator was `|`, which also occurs
   inside the code being mutated (`tab.conflict || tab.detached`), so one mutation was being split
   mid-expression.
2. **A mutant that stops aiming after a refactor.** A mutant pointed at a decision that has since
   moved still reports a kill and reads exactly like a healthy one. **Mutants need re-deriving
   after a refactor, not merely re-running** — and when you re-point one, say which decision you
   aimed at.

**It runs the whole suite per mutant, deliberately.** Running a mutant against only the test
written for it tells you *that test passes*; running it against the suite tells you *which test is
carrying the property*, and those differ more often than is comfortable. Optimising this to a
per-mutant test selection is the obvious speed-up, is a reasonable-looking change, and silently
removes the thing the script is for.

**Also read the baseline correctly.** Compare the *set* of failing test names, not the count. With
any test red for an unrelated reason, a mutant can flip one red green and one green red, leave the
count identical, and read as a clean survivor. That has hidden two genuine kills here.

---

## 6. The end-to-end quit gesture: an honest fraction

The one path that matters — *dirty buffer, real keystroke, correct bytes on disk* — is covered in
three of four links. Stating it as a fraction rather than "E2E is unverified" is deliberate: it is
more accurate and it names what is left.

| link | established by |
|---|---|
| keystroke → `ExitRequested` | a real Cmd+Q spot-test against an app with no dirty buffer |
| `ExitRequested` → prevent, bound, exit | `quit.rs` unit tests, plus mutants for every decision in the coordinator, which all die |
| `before-quit` → flush → `document_write` carrying the CAS hash | frontend tests, plus mutants for forcing past a failed CAS and for dropping `expectedHash` |
| `document_write` → bytes on a real file | Rust composition tests driving `QuitCoordinator` and `DocumentStore` against a real temp file |

**Unverified: the join of the two ends** — a real keystroke, in a real WKWebView, on a document
dirtied by real typing. The harness cannot reach it (no backend, never writes to disk, nothing can
emit `app:before-quit`). Increment 10's CLI closes it, because `medd fixture.md` leaves the editor
focused and every step after is keyboard-only.

**When that script is written, the timing constraint is the whole test.** The autosave debounce is
1000ms, so an ordinary autosave writes the keystroke to disk one second later with no quit
involved. Type, pause, quit — and the assertion is already true before Cmd+Q is pressed. It must
land **inside** the debounce window, the elapsed time must be *asserted* rather than assumed, and
the script must be run once against a build with the flush disabled to confirm it can fail at all.

---

## 7. What "green" currently does not mean

Collected, because each has been over-read at least once:

- **130 Rust tests green** does not mean the app runs. Nothing here starts one.
- **137 frontend tests green** does not mean the frontend agrees with Rust. It mostly means the
  frontend agrees with `tauriMock.ts` and with its own hand-built payloads.
- **7 wire pins green** means the field and variant *names* match. Not the values.
- **22 mutants dying** means those 22 decisions are guarded. It says nothing about decisions
  nobody wrote a mutant for.
- **A green harness** means it looks right in Blink.
- **`detached-recovery.test.ts` green** means the defect is still present — those are
  characterisation tests asserting current, wrong behaviour, and they go red when it is fixed.
  That is deliberate; do not "fix" them to match a new implementation without reading their header.
