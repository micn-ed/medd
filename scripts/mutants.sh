#!/usr/bin/env bash
#
# Mutation harness. Breaks one load-bearing decision at a time and checks that the suite notices.
#
# WHY THIS EXISTS. A green suite proves the tests pass; it does not prove they would fail if the
# code were wrong. Those are different claims, and the gap between them is where this project has
# repeatedly found tests that assert nothing: assertions that held against an implementation which
# never ran, and a named end-to-end deliverable that would have passed against a quit flush that
# had been deleted outright. The question that finds them — *would this still pass if the
# mechanism simply did not run?* — is answerable by experiment rather than by review, and this is
# the experiment.
#
# Run it after changing anything in doc.ts, tabs.svelte.ts, document.rs or quit.rs, and whenever a
# refactor moves a decision: a mutant aimed at a decision that has since moved elsewhere still
# reports a kill, and reads exactly like a healthy one. Mutants need RE-DERIVING after a refactor,
# not merely re-running.
#
# WHY IT COMPARES NAMES, NOT COUNTS. Counting failures is only valid against a green baseline. With
# any test red for an unrelated reason, a mutant can flip one red test green and one green test
# red, leave the count identical, and read as a clean survivor. That happened here: it hid two
# genuine kills behind arithmetic that looked fine.
#
# Usage:  scripts/mutants.sh [frontend|rust|all]      (default: all)

set -euo pipefail
cd "$(dirname "$0")/.."

if [ -n "$(git status --porcelain -- src src-tauri)" ]; then
  echo "refusing to run: src/ or src-tauri/ has uncommitted changes."
  echo "this script edits source files in place and restores them from git; it would eat your work."
  exit 1
fi

WHICH="${1:-all}"
TMP="$(mktemp -d)"
FAILURES=0
trap 'git checkout --quiet -- src src-tauri 2>/dev/null || true; rm -rf "$TMP"' EXIT

# Apply a mutation expressed as a python literal replacement. Fails loudly if the target text is
# absent, because a mutation that silently did not apply is a guaranteed false "survivor".
apply() {
  python3 - "$1" "$2" "$3" <<'PY'
import sys
path, old, new = sys.argv[1], sys.argv[2], sys.argv[3]
s = open(path).read()
if old not in s:
    sys.exit(f"mutation target not found in {path}: {old[:70]!r}")
open(path, 'w').write(s.replace(old, new, 1))
PY
}

frontend_fails() { npx vitest run 2>&1 | grep -E "^\s+×" | sed 's/.*× //; s/ [0-9]*ms$//' | sort; }
rust_fails() { cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | grep -E "^test .* FAILED" | sed 's/^test //; s/ \.\.\..*//' | sort; }

check() { # label, baseline-file, current-fails-file
  local killed
  killed=$(comm -13 "$2" "$3" | wc -l | tr -d ' ')
  if [ "$killed" = "0" ]; then
    printf '  SURVIVOR  %s\n' "$1"
    printf '            nothing detects this. the decision is unprotected.\n'
    FAILURES=$((FAILURES + 1))
  else
    printf '  killed=%-3s %s\n' "$killed" "$1"
  fi
}

run_frontend() {
  echo "== frontend =="
  frontend_fails > "$TMP/base"
  [ -s "$TMP/base" ] && echo "  (baseline has $(wc -l < "$TMP/base" | tr -d ' ') red test(s) — comparing sets, not counts)"

  while IFS='|' read -r label file old new; do
    [ -z "${label// }" ] && continue
    case "$label" in \#*) continue ;; esac
    apply "$file" "$old" "$new"
    frontend_fails > "$TMP/mut"
    git checkout --quiet -- src
    check "$label" "$TMP/base" "$TMP/mut"
  done <<'MUTANTS'
flushAutosave is a no-op|src/doc/doc.ts|export function flushAutosave(path: string): void {|export function flushAutosave(path: string): void {\n  if (true) return
the not-dirty guard is dropped|src/doc/doc.ts|  if (tab.currentText === tab.lastSyncedText) return|  // mutant
the conflict/detached guard is dropped|src/doc/doc.ts|  if (tab.conflict || tab.detached) return|  // mutant
the shutdown latch never closes|src/doc/doc.ts|  if (isShuttingDown) return // shutdown latch|  if (false) return // mutant
a write outcome is applied regardless of generation|src/doc/doc.ts|if (currentGeneration(path) === request.generation) {|if (true) {
a quit-time write drops expectedHash|src/doc/doc.ts|      expectedHash: request.expectedHash,|      expectedHash: isShuttingDown ? '' : request.expectedHash,
markSynced is a no-op|src/tabs/tabs.svelte.ts|export function markSynced(path: string, syncedText: string, hash: string): void {|export function markSynced(path: string, syncedText: string, hash: string): void {\n  if (true) return
markSynced records the buffer, not what was written|src/tabs/tabs.svelte.ts|  tab.lastSyncedText = syncedText|  tab.lastSyncedText = tab.currentText
markConflict is a no-op|src/tabs/tabs.svelte.ts|export function markConflict(path: string, diskContent: string, diskHash: string): void {|export function markConflict(path: string, diskContent: string, diskHash: string): void {\n  if (true) return
markDetached is a no-op|src/tabs/tabs.svelte.ts|export function markDetached(path: string): void {|export function markDetached(path: string): void {\n  if (true) return
applyExternalContent is a no-op|src/tabs/tabs.svelte.ts|export function applyExternalContent(path: string, newContent: string, newHash: string): void {|export function applyExternalContent(path: string, newContent: string, newHash: string): void {\n  if (true) return
resolveConflictKeepMine is a no-op|src/tabs/tabs.svelte.ts|export function resolveConflictKeepMine(path: string): void {|export function resolveConflictKeepMine(path: string): void {\n  if (true) return
MUTANTS
}

run_rust() {
  echo "== rust =="
  rust_fails > "$TMP/base"

  while IFS='|' read -r label file old new; do
    [ -z "${label// }" ] && continue
    case "$label" in \#*) continue ;; esac
    apply "$file" "$old" "$new"
    rust_fails > "$TMP/mut"
    git checkout --quiet -- src-tauri
    check "$label" "$TMP/base" "$TMP/mut"
  done <<'MUTANTS'
compare-and-swap no longer rejects a stale hash|src-tauri/src/document.rs|        if current_hash != *expected_hash {|        if false && current_hash != *expected_hash {
a repeat exit request is let through (the reversed-away rule)|src-tauri/src/quit.rs|            start_flush: self.begin_shutdown(),|            start_flush: { let s = self.begin_shutdown(); if s.is_none() { return ExitDecision { prevent: false, start_flush: None } } s },
decide() ignores ready_to_exit (the app cannot be quit)|src-tauri/src/quit.rs|        if self.is_ready_to_exit() {|        if false && self.is_ready_to_exit() {
every exit request starts its own flush thread|src-tauri/src/quit.rs|        if *shutting_down {\n            return None;\n        }|        if false {\n            return None;\n        }
mark_ready_to_exit does nothing|src-tauri/src/quit.rs|        *self.ready_to_exit.lock().unwrap() = true;|        // mutant
the quit ceiling never waits|src-tauri/src/quit.rs|    rx.recv_timeout(ceiling).is_ok()|    let _ = (rx, ceiling); false
signal_ready does nothing|src-tauri/src/quit.rs|        if let Some(tx) = self.tx.lock().unwrap().take() {\n            let _ = tx.send(());\n        }|        let _ = self.tx.lock().unwrap().take();
MUTANTS

  # NOTE: there is deliberately no mutant replacing the bounded wait with an unbounded `recv()`.
  # It is caught, but by hanging rather than failing — `returns_false_once_the_ceiling_elapses`
  # keeps its sender alive on purpose, so a wait that lost its bound blocks there forever. Running
  # it here would hang this script rather than report. If the Rust suite ever hangs instead of
  # failing, that is the first place to look.
}

case "$WHICH" in
  frontend) run_frontend ;;
  rust) run_rust ;;
  all) run_frontend; run_rust ;;
  *) echo "usage: $0 [frontend|rust|all]"; exit 2 ;;
esac

echo
if [ "$FAILURES" -gt 0 ]; then
  echo "$FAILURES surviving mutant(s): a decision changed and no test noticed."
  exit 1
fi
echo "no survivors — every mutated decision was detected."
