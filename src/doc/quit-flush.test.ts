// The quit flush and shutdown latch (plan-v0.1.md's fifth blocker). The latch this exercises
// (`isShuttingDown` in doc.ts) is one-way by design -- there is no "after" to reset it to, since
// the process is exiting once it's set -- so every test re-imports doc.ts *and* tabs.svelte.ts
// together via `vi.resetModules()` rather than reusing a shared instance. Both, deliberately: doc.ts
// imports tabs.svelte.ts internally, so resetting only doc.ts would leave it talking to a second,
// disconnected copy of the tab state this file's own `openTab`/`getTab` calls operate on.
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import { EditorView } from '@codemirror/view'
import type { EditorState } from '@codemirror/state'

const { invokeMock, listeners } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((event: string, handler: (e: { payload: unknown }) => void) => {
    listeners.set(event, handler)
    return Promise.resolve(() => {})
  }),
}))

function fireBackendEvent(name: string, payload?: unknown) {
  listeners.get(name)?.({ payload })
}

// The listener's own promise chain (waitForAllQuiescent -> settleIfQuiescent's waiters -> the
// listener's .then(() => invoke('quit_ready'))) is several microtask hops deep and is
// deliberately fire-and-forget from `initDocSync`'s side (nothing here can `await` it directly),
// so tests drain the microtask queue generously rather than guessing an exact hop count.
async function flushMicrotasks(times = 12): Promise<void> {
  for (let i = 0; i < times; i++) {
    await Promise.resolve()
  }
}

let openTab: (path: string, content: string, hash: string, workspaceRoot: string | null) => void
let getTab: (path: string) => { currentText: string } | undefined
let editorStateFor: (path: string) => EditorState

function edit(path: string, insert: string) {
  const view = new EditorView({ state: editorStateFor(path) })
  view.dispatch({ changes: { from: view.state.doc.length, insert } })
  view.destroy()
}

beforeEach(async () => {
  vi.useFakeTimers()
  invokeMock.mockReset()
  listeners.clear()
  // Fresh instances of both modules together (see the file header for why both, not just doc.ts).
  vi.resetModules()
  const tabs = await import('../tabs')
  const doc = await import('./doc')
  ;({ openTab, getTab, editorStateFor } = tabs)
  doc.initDocSync()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('app:before-quit', () => {
  test('flushes every open tab and reports done once everything is quiescent', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    edit('/workspace/a.md', 'X') // starts a debounce -- something for the flush to find
    openTab('/workspace/b.md', 'v1', 'h1', '/workspace')
    edit('/workspace/b.md', 'Y')
    invokeMock.mockResolvedValue('h2')

    fireBackendEvent('app:before-quit')

    // flushAll -> flushAutosave issues both writes synchronously (clearing the pending timer and
    // calling requestWrite directly) -- no timer advance needed to observe the invoke calls.
    expect(invokeMock).toHaveBeenCalledWith('document_write', {
      path: '/workspace/a.md',
      content: 'v1X',
      expectedHash: 'h1',
    })
    expect(invokeMock).toHaveBeenCalledWith('document_write', {
      path: '/workspace/b.md',
      content: 'v1Y',
      expectedHash: 'h1',
    })

    // Let both writes' promises settle and the waitForAllQuiescent chain finish.
    await flushMicrotasks()

    expect(invokeMock).toHaveBeenCalledWith('quit_ready')
  })

  test('reports done immediately when nothing was dirty', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace') // never edited -- nothing to flush

    fireBackendEvent('app:before-quit')
    await flushMicrotasks()

    expect(invokeMock).toHaveBeenCalledWith('quit_ready')
    expect(invokeMock).not.toHaveBeenCalledWith('document_write', expect.anything())
  })
})

describe('the shutdown latch: no new work of any kind once quitting has begun', () => {
  test('a later edit does not schedule a new autosave', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace') // clean -- the flush is a no-op

    fireBackendEvent('app:before-quit')
    await Promise.resolve()

    edit('/workspace/a.md', 'X') // "new work" arriving after the latch closed
    await vi.advanceTimersByTimeAsync(5000)

    expect(invokeMock).not.toHaveBeenCalledWith('document_write', expect.anything())
  })

  test('an external change to a clean tab is ignored, not silently reloaded', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')

    fireBackendEvent('app:before-quit')
    await Promise.resolve()

    fireBackendEvent('document:changed-on-disk', {
      path: '/workspace/a.md',
      content: 'external content',
      hash: 'hExternal',
    })

    // Not applyExternalContent's doing -- untouched, not reloaded to "external content".
    expect(getTab('/workspace/a.md')?.currentText).toBe('v1')
  })
})

describe('a rejected compare-and-swap during the flush', () => {
  test('drops that write rather than forcing it, and the flush still completes', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    edit('/workspace/a.md', 'X')
    invokeMock.mockRejectedValue({ kind: 'Conflict', currentContent: 'disk', hash: 'hDisk' })

    fireBackendEvent('app:before-quit')

    await flushMicrotasks()

    // The rejected attempt is the only document_write call -- nothing retries it, and nothing
    // else writes in its place. Quitting must never force a write past a failed CAS: that would
    // silently overwrite whatever the external change was, on the one path with no banner to
    // show it (architecture.md §3; docs/design-quit-flush.md §5).
    expect(invokeMock.mock.calls.filter(([cmd]) => cmd === 'document_write')).toHaveLength(1)
    // And the rejection must not hang the flush waiting for a write that will never land.
    expect(invokeMock).toHaveBeenCalledWith('quit_ready')
  })
})
