// Acceptance tests for the quit flush, complementing `quit-flush.test.ts` rather than repeating
// it. That file covers the happy path — every open tab flushed, done reported once quiescent,
// nothing rescheduled afterwards. These cover the cases where the flush must *decline* to act,
// and the case where something keeps arriving while it drains.
//
// The criteria they come from were fixed before the code existed (docs/qa-increment-7.md §13):
// the exit must issue everything the debounce still owes, and the outcome of those writes applies
// to nothing. Two of them exist because the natural shortcut at shutdown — "it's a quit, just
// force the write through" — is wrong in an asymmetric way: losing an edit is recoverable because
// the user still has their file in front of them, while silently overwriting someone else's
// change is not, and at quit there is no banner and nobody watching.
//
// ISOLATION, same as `quit-flush.test.ts` and for the same reason: `isShuttingDown` is one-way by
// design, so every test re-imports doc.ts *and* tabs.svelte.ts together via `vi.resetModules()`.
// Resetting one without the other leaves them operating on disconnected copies of tab state.
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import { EditorView } from '@codemirror/view'

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

let tabs: typeof import('../tabs')
let doc: typeof import('./doc')

const fire = (n: string, p?: unknown) => listeners.get(n)?.({ payload: p })
function edit(path: string, insert: string) {
  const v = new EditorView({ state: tabs.editorStateFor(path) })
  v.dispatch({ changes: { from: v.state.doc.length, insert } })
  v.destroy()
}
const open = (p: string, c: string, h: string) => tabs.openTab(p, c, h, '/w')
const writes = () =>
  invokeMock.mock.calls
    .filter((c) => c[0] === 'document_write')
    .map((c) => ({ content: c[1].content, expectedHash: c[1].expectedHash }))
const readySignals = () => invokeMock.mock.calls.filter((c) => c[0] === 'quit_ready').length

beforeEach(async () => {
  vi.useFakeTimers()
  invokeMock.mockReset()
  invokeMock.mockResolvedValue('hNew')
  listeners.clear()
  vi.resetModules()
  tabs = await import('../tabs')
  doc = await import('./doc')
  doc.initDocSync()
})
afterEach(() => vi.useRealTimers())

describe('a quit-time write must be able to be rejected', () => {
  test('the flush carries expectedHash, so the compare-and-swap still applies', async () => {
    // A shortcut write that skipped the CAS would satisfy every other requirement of the quit
    // flush while silently overwriting an external change — on the one path where the user is
    // not watching and no banner can be shown. Carrying the hash is the observable proof that
    // the ordinary write path, and therefore the CAS, is still in play.
    open('/w/a.md', 'v1', 'h1')
    edit('/w/a.md', 'X')
    fire('app:before-quit')
    await vi.advanceTimersByTimeAsync(0)

    expect(writes()).toEqual([{ content: 'v1X', expectedHash: 'h1' }])
  })
})

describe('quitting does not resolve a conflict in either direction', () => {
  test('a conflicted tab is not flushed', async () => {
    open('/w/a.md', 'v1', 'h1')
    edit('/w/a.md', 'MINE')
    fire('document:changed-on-disk', { path: '/w/a.md', content: 'THEIRS', hash: 'hDisk' })
    expect(tabs.getTab('/w/a.md')!.conflict).not.toBeNull() // the mechanism actually ran

    fire('app:before-quit')
    await vi.advanceTimersByTimeAsync(0)
    expect(writes()).toHaveLength(0)
  })

  test('a detached tab is not flushed — quitting never recreates a deleted file', async () => {
    open('/w/a.md', 'v1', 'h1')
    edit('/w/a.md', 'MINE')
    fire('document:removed-on-disk', { path: '/w/a.md' })
    expect(tabs.getTab('/w/a.md')!.detached).toBe(true) // the mechanism actually ran

    fire('app:before-quit')
    await vi.advanceTimersByTimeAsync(0)
    expect(writes()).toHaveLength(0)
  })

  test('a suspended tab does not stop the others from flushing', async () => {
    open('/w/a.md', 'a', 'ha')
    open('/w/b.md', 'b', 'hb')
    edit('/w/a.md', 'A')
    edit('/w/b.md', 'B')
    fire('document:changed-on-disk', { path: '/w/a.md', content: 'THEIRS', hash: 'hDisk' })

    fire('app:before-quit')
    await vi.advanceTimersByTimeAsync(0)
    expect(writes().map((w) => w.content)).toEqual(['bB'])
  })
})

describe('the latch, with something actually arriving mid-drain', () => {
  // A quiet drain cannot distinguish a latch that works from a ceiling that is quietly ending it.
  // Only an event arriving *during* the drain can, which is why this provokes one rather than
  // trusting the silence — without the latch, every reload would run back through onDocChanged
  // and push quiescence a second further away, indefinitely.
  test('a storm of watcher events during the drain cannot hold quiescence open', async () => {
    open('/w/a.md', 'v1', 'h1')
    edit('/w/a.md', 'X')
    fire('app:before-quit')

    for (let i = 0; i < 20; i++) {
      fire('document:changed-on-disk', { path: '/w/a.md', content: `v${i}`, hash: `h${i}` })
      await vi.advanceTimersByTimeAsync(100)
    }
    await vi.advanceTimersByTimeAsync(2000)

    expect(readySignals()).toBeGreaterThan(0)
  })
})
