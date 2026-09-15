// CHARACTERISATION TESTS for an OPEN defect — review finding 4, `detached` is terminal.
//
// These assert what medd does today, which is not what it should do. They are green, and they go
// red the moment the defect is fixed. That is deliberate and it is the whole point: the fix
// becomes a visible, mechanical diff (flip three `true`s to `false`s and delete this header)
// rather than something that could land unnoticed.
//
// WHY NOT `test.fails()`. It was the obvious choice and it is the wrong one. `test.fails()`
// passes when the body throws *for any reason whatsoever* — verified: a genuine assertion
// failure, a call to an undefined function, and a `throw` in setup are all reported identically
// as "expected fail". So it certifies "something went wrong in here", not "this behaviour is
// broken in the way described". A rename or a signature change would rot the test into failing
// for an unrelated reason, `test.fails()` would keep reporting green, and — because a green
// expected-fail is exactly what you want to see — nobody would look. Worse, its one advantage
// evaporates too: a test already failing for an unrelated reason keeps failing after the defect
// is fixed, so it never announces the fix either.
//
// A plain assertion on the current value has none of that. It fails specifically, on a value
// mismatch, and only when the behaviour actually changes.
//
// THE DEFECT, for whoever comes to fix it: `markDetached` sets `tab.detached = true` and nothing
// anywhere sets it back. Autosave is suspended for that tab permanently. The file returning to
// disk does not clear it (Rust stopped tracking the path on removal, so no `changed-on-disk`
// event is ever emitted for it), and neither does re-opening it from the tree (`openTab`
// early-returns for a path that is already open, discarding the freshly read content *and* hash).
// The only exit is closing the tab, which under P-2 is "always safe" and takes the buffer with it.
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import { EditorView } from '@codemirror/view'
import { closeAllTabs, editorStateFor, getTab, openTab } from '../tabs'

const { invokeMock, listeners } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listeners: new Map<string, (e: { payload: unknown }) => void>(),
}))
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((e: string, h: (x: { payload: unknown }) => void) => {
    listeners.set(e, h)
    return Promise.resolve(() => {})
  }),
}))
const { initDocSync } = await import('./doc')

const fire = (n: string, p: unknown) => listeners.get(n)?.({ payload: p })
function edit(path: string, insert: string) {
  const v = new EditorView({ state: editorStateFor(path) })
  v.dispatch({ changes: { from: v.state.doc.length, insert } })
  v.destroy()
}

beforeEach(() => {
  vi.useFakeTimers()
  closeAllTabs()
  invokeMock.mockReset()
  invokeMock.mockResolvedValue('h2')
  listeners.clear()
  initDocSync()
})
afterEach(() => vi.useRealTimers())

describe('finding 4 (OPEN): a detached tab never recovers', () => {
  test('the file returning to disk does NOT clear detached — should be false once fixed', () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    fire('document:removed-on-disk', { path: '/workspace/a.md' })
    expect(getTab('/workspace/a.md')!.detached).toBe(true)

    // git checkout, stash pop, or the file restored by any other means.
    fire('document:changed-on-disk', { path: '/workspace/a.md', content: 'v1', hash: 'h1' })

    expect(getTab('/workspace/a.md')!.detached).toBe(true) // ← should be false once fixed
  })

  test('re-opening from the tree does NOT clear detached — should be false once fixed', () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    edit('/workspace/a.md', 'important work')
    fire('document:removed-on-disk', { path: '/workspace/a.md' })

    // The user restores the file and clicks it again. App.svelte's openFile reads it afresh and
    // calls openTab with the new content and hash; openTab discards both.
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')

    expect(getTab('/workspace/a.md')!.detached).toBe(true) // ← should be false once fixed
  })

  test('typing into a detached tab is silently never written — should write once fixed', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    fire('document:removed-on-disk', { path: '/workspace/a.md' })

    // An hour of work, with only a small ⚠ in the tab strip to say it is going nowhere.
    edit('/workspace/a.md', ' a lot of new writing')
    await vi.advanceTimersByTimeAsync(60_000)

    expect(invokeMock).not.toHaveBeenCalled() // ← should have been called once fixed
  })
})
