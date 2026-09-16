# medd — for a Claude session picking this up

A resident macOS Markdown editor with live side-by-side preview. Rust + Tauri v2, CodeMirror 6,
markdown-it, Svelte 5. Local-first: no server, no account, no sync. macOS only for v1.

**Read [docs/handoff.md](docs/handoff.md) before doing anything.** It is the whole picture. This
file is only what you need in context from the first message.

## The four things that will save you the most time

1. **[docs/conventions.md](docs/conventions.md) is not style advice.** Every rule in it exists
   because something went wrong here, and the incident is recorded with it. Read it before writing
   a test or a fix — most of what has gone wrong on this project was caught by one of those rules,
   and most of what got through was a rule nobody had written yet.

2. **Work in your own `git worktree`.** Not the shared checkout. `git worktree add ../medd-<you>`.
   A written convention against mutating a shared tree was violated twice within hours by two
   people who had both read it, and someone's uncommitted work vanished. Structure, not attention.

3. **Verify results; do not trust exit codes or summaries.** A build here failed, reported success,
   and left a stale artifact that looked new — it was caught by checking a file's timestamp. A test
   harness exited 0 having done nothing. A scripted edit silently applied nothing. This happens
   often enough to assume it.

4. **State what a fix must guarantee before writing the fix.** Two sessions once produced fixes
   that would have silently cancelled each other; described as code changes they looked
   complementary, described as invariants the conflict was obvious and named its own resolution.

## Who does what

Sessions coordinate by `SendMessage`. **Everything routes through the leader** — sessions do not
negotiate with each other directly, and the manager receives short executive summaries only.

| role | owns |
|---|---|
| **manager** | CEO/CTO. Strategy only. Gets a few bullets, not detail. |
| **leader** | Coordinates, decides, routes, merges, pushes. Every ruling. |
| **planner-ba** | Requirements. |
| **researcher** | Technical research with live sources. |
| **principal-architect** | Design. Validates *before* implementation, not after. |
| **dev** | Writes code, and writes its tests. |
| **qa** | Independent verification. Acceptance, not authorship. |

**Authorship and acceptance are separate.** Whoever writes an increment writes its tests — that is
how correct code gets written, not a verification step. Acceptance is somebody else's.

## Where things are

- `docs/handoff.md` — full context, history, how to work here
- `docs/todo.md` — ready-to-execute roadmap, no archaeology required
- `docs/verification-status.md` — **read §1 and §2 before you read a green test suite.** What each
  instrument here can and cannot establish. 130 passing Rust tests are weaker evidence than they
  look, in ways that are not obvious.
- `docs/plan-v0.1.md` — the twelve increments and their status
- `docs/architecture.md` — how it is built, and why
- `docs/decisions.md` — D-1…D-16, **locked**. Code that diverges from one is a finding, not a
  licence to edit the decision.
- `docs/adr/` — the three choices that needed their own argument

## Commands

```sh
git worktree add ../medd-<you>          # do this first
cd src-tauri && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check
npx vitest run && npx svelte-check --tsconfig ./tsconfig.app.json
./scripts/mutants.sh                    # mutation harness — read its header first
npm run harness                         # frontend in a browser, fixture workspace
npm run tauri build                     # the real .app
```

Rust is pinned via `rust-toolchain.toml` and needs rustup, not a package manager.

**Commit with explicit paths on the commit itself** — `git commit -m "…" -- <paths>`, message
before the pathspec. `git add <path>` then `git commit` commits the *whole index*, including
another session's staged work.
