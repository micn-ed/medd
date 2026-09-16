# medd — crew handoff

For a Claude crew picking this project up cold. Read this before writing anything.

Start with [CLAUDE.md](../CLAUDE.md) if you haven't — it's the short version and it auto-loads.

---

## 1. What medd is

A resident macOS Markdown editor with live side-by-side preview. Open a folder, browse it, edit a
document, watch it render, and trust that it's on disk — without ever pressing save. Confluence-like
in feel: documents are pleasant to read, structured content renders properly, and the app stays
open rather than being launched per task.

Local-first and filesystem-backed. No server, no account, no sync. macOS only for v1.

Rust + Tauri v2 (system WebView), CodeMirror 6, markdown-it, Svelte 5. Public, GPL-3.0.

**The architectural line everything hangs from:** Rust owns the filesystem and the process; the
WebView owns the document. A keystroke never crosses the IPC bridge. Only three things do — a load,
a save, and being told the file changed underneath us.

---

## 2. How the crew is organised

Seven roles across seven sessions. The structure exists because two of these roles were added
mid-project and immediately started finding things the others couldn't.

| role | owns | does not |
|---|---|---|
| **manager** | Strategy. Ship/no-ship, scope, headcount. | Receive detail. A few bullets, outcomes not mechanisms. |
| **leader** | Coordination, every ruling, merges, pushes, what ships. | Write features. |
| **planner-ba** | Requirements. | Design. |
| **researcher** | Technical research against live sources. | Decide. |
| **principal-architect** | Design. Validates **before** implementation. | Write features. |
| **dev** | Code, and the tests that make it correct. | Accept its own work. |
| **qa** | Independent verification and acceptance. | Fix what it finds. |

### The two role boundaries that matter

**Authorship and acceptance are separate.** Whoever writes an increment writes its tests — that is
how correct code gets written, not a verification step, and in practice it is where nearly every
real defect here was caught. Acceptance is somebody else's: an independent pass against the
increment's definition of done. Collapsing them loses the half that finds things.

**Design is validated before implementation, not after.** The ahead-of-implementation review has
paid five times. Twice it found an increment that could not be built as specified — once because
the primitive it depended on couldn't do what the spec claimed, once because a platform event
never fires on the gesture the design assumed. Both were found before a line was written.

### How sessions talk

`SendMessage`, and **everything routes through the leader.** Sessions don't negotiate with each
other directly. This isn't ceremony — it's what makes a single person responsible for every
decision, and it's why contradictions between two sessions surface as a ruling rather than as
whoever committed last.

The leader gives the manager **executive summaries only**: outcomes, risk, and anything strategic.
Not mechanisms.

---

## 3. The conventions, and why they're not style advice

[conventions.md](conventions.md) is the most important document here after this one. **Every rule
in it exists because something went wrong on this project, and the incident is recorded with the
rule.** That matters: a rule you can argue with is a rule you can apply correctly to a case it
didn't anticipate.

The recurring theme, across almost every incident: **a result that looks better than the truth, and
therefore stops the search.** A test that passes because the mechanism never ran. A harness that
exits 0 having done nothing. A build that reports success and leaves a stale artifact. A mutant
that "survived" because the mutation failed to apply. A borrowed function signature that reads as
a fix and guarantees nothing. Six or seven distinct faces of one failure.

The ones that would cost a newcomer the most:

- **State what a fix must guarantee, before writing the fix.** Two sessions once produced fixes
  that would have silently cancelled each other. As code changes they read as complementary; as
  invariants the conflict was obvious and named its own resolution.
- **Assert on values, not existence — and ask *would this test still pass if the mechanism never
  ran?*** That question, run as mutation rather than opinion, found four vacuous tests in one pass,
  including two that had been reported as positive evidence.
- **Work in your own worktree.** A written convention against mutating a shared checkout was
  violated twice within hours, by two people who had both read it, and someone's uncommitted work
  vanished. Structure beats attention.
- **Verify the artifact, not the description of it.** A summary can be honest while the thing it
  describes overclaims — they are different objects and only one of them ships.
- **Operational documents reference; they do not restate.** A plan that restates a mechanism goes
  stale invisibly, because the stale row still reads as current — it's about the right subject.

---

## 4. Where the project is

**Nine of twelve increments done.** See [plan-v0.1.md](plan-v0.1.md) for all twelve and their
status, and [todo.md](todo.md) for what to do next.

Working today: folder workspace with a file tree, tabs, a CodeMirror source pane, live preview,
reading mode, quick-open, debounced autosave with atomic writes, external-change detection with a
conflict banner, Cmd+W closing a tab, and quitting flushing pending work.

Not yet: the CLI (built, on an unmerged branch), session persistence, large-document degradation,
and Finder integration (v0.2).

**Before you read a green test suite here, read
[verification-status.md](verification-status.md) §1 and §2.** They change how you should interpret
one. 130 Rust and 137 frontend tests pass; what that does and doesn't establish is not obvious, and
the document is explicit about the parts no instrument here can reach.

### The five blockers, because they explain the culture

All five were data loss, all were in code that passed its tests, and none were found by running
them:

1. Windows line endings **corrupted and the corruption saved over the file**.
2. "Keep mine" on a conflict **often saved nothing at all**.
3. Typing, then Cmd+W within a second, **lost that second**.
4. A conflict warning **on a document nobody had edited**.
5. **Quitting lost pending work in every open tab** — behind Cmd+W, which everywhere else means
   *close tab*.

Two more were found by a person using the app for ten minutes: the folder picker **deadlocked the
whole app on click**, and a rejected save **showed no conflict banner at all**. Neither was
reachable by any test on this project, and the second was hidden because six test files mocked the
shape the frontend wanted rather than the shape Rust sent — green tests proving the frontend agreed
with itself.

**The lesson worth carrying: this project's testing is unusually rigorous and it did not catch the
two bugs a user hit in ten minutes.** Both classes are now guarded, but the general point stands —
someone has to actually use it.
