// QA re-verification: does flush-on-close compose with drop-stale-outcome?
//
// Written against the CONTRACT, not the implementation, so it survives whatever shape the fix
// takes. The invariant under test has three clauses:
//   (1) a close issues everything the debounce still owes;
//   (2) a write's outcome is applied only to the state that issued it;
//   (3) issuing and applying are INDEPENDENT — dropping a flush's outcome must not cancel
//       the flush. Clause 3 is the one an implementation is most likely to lose, because the
//       natural way to write clause 2 ("if the tab is gone, bail") also cancels clause 1.
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import { EditorView } from '@codemirror/view'
import {
  closeAllTabs, closeTab, editorStateFor, getTab, openTab,
} from '../tabs'

const { invokeMock, listeners } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listeners: new Map<string, (e: { payload: unknown }) => void>(),
}))
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((e: string, h: (x: { payload: unknown }) => void) => {
    listeners.set(e, h); return Promise.resolve(() => {})
  }),
}))
const doc = await import('./doc')
const fire = (n: string, p: unknown) => listeners.get(n)?.({ payload: p })

function edit(path: string, insert: string) {
  const v = new EditorView({ state: editorStateFor(path) })
  v.dispatch({ changes: { from: v.state.doc.length, insert } })
  v.destroy()
}
/** A write whose settling we control, so "in flight" is a real state and not a race. */
function deferredWrite() {
  let settle!: (hash: string) => void
  let reject!: (e: unknown) => void
  const gate = new Promise<string>((res, rej) => { settle = res; reject = rej })
  invokeMock.mockImplementationOnce(() => gate)
  return { settle, reject }
}
const writes = () => invokeMock.mock.calls
  .filter((c) => c[0] === 'document_write')
  .map((c) => ({ path: c[1].path, content: c[1].content, expectedHash: c[1].expectedHash }))

beforeEach(() => {
  vi.useFakeTimers(); closeAllTabs(); invokeMock.mockReset()
  invokeMock.mockResolvedValue('hDefault'); listeners.clear(); doc.initDocSync()
})
afterEach(() => vi.useRealTimers())

// ---------------------------------------------------------------- clause 1

describe('clause 1 — a close issues everything the debounce still owes', () => {
  test('a pending debounce timer is flushed, not dropped', async () => {
    openTab('/w/a.md', 'v1', 'h1', '/w')
    edit('/w/a.md', 'X')
    closeTab('/w/a.md')
    await vi.advanceTimersByTimeAsync(5000)
    expect(writes()).toEqual([{ path: '/w/a.md', content: 'v1X', expectedHash: 'h1' }])
  })

  test('closeAllTabs flushes every tab', async () => {
    openTab('/w/a.md', 'a', 'ha', '/w')
    openTab('/w/b.md', 'b', 'hb', '/w')
    edit('/w/a.md', 'A'); edit('/w/b.md', 'B')
    closeAllTabs()
    await vi.advanceTimersByTimeAsync(5000)
    expect(writes().map((w) => w.content).sort()).toEqual(['aA', 'bB'])
  })

  test('a close with an in-flight write and nothing newer does not double-write', async () => {
    openTab('/w/a.md', 'v1', 'h1', '/w')
    edit('/w/a.md', 'X')
    const w = deferredWrite()
    await vi.advanceTimersByTimeAsync(1000)   // the debounce fires; write in flight
    expect(writes()).toHaveLength(1)

    closeTab('/w/a.md')                        // nothing further is owed
    w.settle('h2')
    await vi.advanceTimersByTimeAsync(5000)
    expect(writes()).toHaveLength(1)
  })

  // THE THREE-WAY ROUTE. The second edit never gets its own timer — it is absorbed into the
  // retry queued behind the in-flight write — so flushing timers alone cannot save it.
  test('a close with an in-flight write AND a retry queued behind it writes the queued text', async () => {
    openTab('/w/a.md', 'v1', 'h1', '/w')
    edit('/w/a.md', 'X')
    const w = deferredWrite()
    await vi.advanceTimersByTimeAsync(1000)   // write #1 in flight, content "v1X"

    edit('/w/a.md', 'Y')                       // now "v1XY"
    await vi.advanceTimersByTimeAsync(1000)   // this tick queues a retry; it does not write

    closeTab('/w/a.md')
    w.settle('h2')
    await vi.advanceTimersByTimeAsync(5000)

    expect(writes()).toHaveLength(2)
    // The queued edit must reach disk, against the baseline the first write established.
    expect(writes()[1]).toEqual({ path: '/w/a.md', content: 'v1XY', expectedHash: 'h2' })
  })
})

// ---------------------------------------------------------------- clause 2

describe('clause 2 — a write outcome applies only to the state that issued it', () => {
  test('Reload during an in-flight write survives that write settling', async () => {
    openTab('/w/a.md', 'v1', 'h1', '/w')
    edit('/w/a.md', 'X')
    const w = deferredWrite()
    await vi.advanceTimersByTimeAsync(1000)

    fire('document:changed-on-disk', { path: '/w/a.md', content: 'DISK', hash: 'hDISK' })
    doc.reload('/w/a.md')
    const resolved = { ...getTab('/w/a.md')! }

    w.settle('hSTALE')
    await vi.advanceTimersByTimeAsync(5000)

    const t = getTab('/w/a.md')!
    expect(t.lastSyncedText).toBe(resolved.lastSyncedText)
    expect(t.expectedHash).toBe(resolved.expectedHash)
    expect(t.currentText).toBe(t.lastSyncedText)   // still clean
    expect(t.conflict).toBeNull()
  })

  test('close-then-reopen during an in-flight write does not fabricate a conflict', async () => {
    openTab('/w/a.md', 'v1', 'h1', '/w')
    edit('/w/a.md', 'X')
    const w = deferredWrite()
    await vi.advanceTimersByTimeAsync(1000)

    closeTab('/w/a.md')
    openTab('/w/a.md', 'FRESH FROM DISK', 'hFRESH', '/w')

    w.settle('hSTALE')
    await vi.advanceTimersByTimeAsync(5000)

    const t = getTab('/w/a.md')!
    expect(t.currentText).toBe('FRESH FROM DISK')
    expect(t.lastSyncedText).toBe('FRESH FROM DISK')
    expect(t.expectedHash).toBe('hFRESH')
    expect(t.conflict).toBeNull()               // no banner on a document nobody edited
  })

  test('a stale write REJECTED after a resolution does not re-raise the banner', async () => {
    openTab('/w/a.md', 'v1', 'h1', '/w')
    edit('/w/a.md', 'X')
    const w = deferredWrite()
    await vi.advanceTimersByTimeAsync(1000)

    fire('document:changed-on-disk', { path: '/w/a.md', content: 'DISK', hash: 'hDISK' })
    doc.reload('/w/a.md')

    w.reject({ kind: 'Conflict', currentContent: 'DISK', hash: 'hDISK' })
    await vi.advanceTimersByTimeAsync(5000)

    expect(getTab('/w/a.md')!.conflict).toBeNull()
  })
})

// ---------------------------------------------------------------- clause 3

describe('clause 3 — issuing and applying are independent', () => {
  // The direct test. After a close there is no tab, so the outcome MUST be dropped; the write
  // itself must still have happened. An implementation that expresses clause 2 as "no tab, no
  // write" passes clause 2 and silently fails this.
  test('a flush still writes even though nothing remains to apply its outcome to', async () => {
    openTab('/w/a.md', 'v1', 'h1', '/w')
    edit('/w/a.md', 'X')
    closeTab('/w/a.md')
    await vi.advanceTimersByTimeAsync(5000)

    expect(getTab('/w/a.md')).toBeUndefined()      // nothing to apply to
    expect(writes()).toHaveLength(1)               // and yet the edit reached disk
  })

  test('a flush whose write is rejected does not throw and does not resurrect state', async () => {
    openTab('/w/a.md', 'v1', 'h1', '/w')
    edit('/w/a.md', 'X')
    invokeMock.mockReset()
    invokeMock.mockRejectedValue({ kind: 'Conflict', currentContent: 'DISK', hash: 'hDISK' })
    closeTab('/w/a.md')
    await expect(vi.advanceTimersByTimeAsync(5000)).resolves.not.toThrow()
    // Asserted explicitly, so this cannot pass by virtue of no flush having happened — which is
    // how it would pass today, and would keep passing if clause 1 were quietly dropped.
    expect(writes()).toHaveLength(1)
    expect(getTab('/w/a.md')).toBeUndefined()
  })

  test('dropping outcomes does not suppress a later legitimate write', async () => {
    openTab('/w/a.md', 'v1', 'h1', '/w')
    edit('/w/a.md', 'X')
    closeTab('/w/a.md')
    await vi.advanceTimersByTimeAsync(5000)
    const afterClose = writes().length

    openTab('/w/a.md', 'v2', 'h2', '/w')
    edit('/w/a.md', 'Z')
    await vi.advanceTimersByTimeAsync(5000)

    expect(writes().length).toBe(afterClose + 1)
    expect(writes().at(-1)).toEqual({ path: '/w/a.md', content: 'v2Z', expectedHash: 'h2' })
  })
})
