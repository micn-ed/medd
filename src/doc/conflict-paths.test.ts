// QA: are the watcher-raised and CAS-raised conflicts genuinely the same state?
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
    listeners.set(e, h); return Promise.resolve(() => {})
  }),
}))
const { initDocSync, keepMine } = await import('./doc')
const fire = (n: string, p: unknown) => listeners.get(n)?.({ payload: p })
function edit(path: string, insert: string) {
  const v = new EditorView({ state: editorStateFor(path) })
  v.dispatch({ changes: { from: v.state.doc.length, insert } }); v.destroy()
}
const snap = (p: string) => { const t = getTab(p)!; return {
  currentText: t.currentText, lastSyncedText: t.lastSyncedText,
  expectedHash: t.expectedHash, conflict: t.conflict, detached: t.detached } }

beforeEach(() => { vi.useFakeTimers(); closeAllTabs(); invokeMock.mockReset(); listeners.clear(); initDocSync() })
afterEach(() => vi.useRealTimers())

describe('the two ways a conflict can arise', () => {
  test('produce identical tab state', async () => {
    // Path A: the watcher tells us first.
    invokeMock.mockResolvedValue('hX')
    openTab('/w/a.md', 'v1', 'h1', '/w')
    edit('/w/a.md', 'MINE')
    fire('document:changed-on-disk', { path: '/w/a.md', content: 'THEIRS', hash: 'hDisk' })
    const viaWatcher = snap('/w/a.md')

    // Path B: our own autosave loses the CAS race.
    closeAllTabs(); invokeMock.mockReset()
    invokeMock.mockRejectedValue({ kind: 'Conflict', currentContent: 'THEIRS', hash: 'hDisk' })
    openTab('/w/a.md', 'v1', 'h1', '/w')
    edit('/w/a.md', 'MINE')
    await vi.advanceTimersByTimeAsync(1500)
    const viaCas = snap('/w/a.md')

    // Both snapshots must actually describe a conflict. Without this the test passes when
    // markConflict is a no-op, because two identical no-conflict states compare equal too —
    // a test named for conflicts, green in a world with no conflicts. Verified by mutation.
    expect(viaWatcher.conflict).toEqual({ diskContent: 'THEIRS', diskHash: 'hDisk' })
    expect(viaCas).toEqual(viaWatcher)
  })
})

describe('Keep mine, step by step', () => {
  test('adopts the disk hash, leaves the buffer alone, and writes only on the next tick', async () => {
    invokeMock.mockResolvedValue('hNew')
    openTab('/w/a.md', 'v1', 'h1', '/w')
    edit('/w/a.md', 'MINE')
    fire('document:changed-on-disk', { path: '/w/a.md', content: 'THEIRS', hash: 'hDisk' })

    const atBanner = snap('/w/a.md')
    expect(atBanner.currentText).toBe('v1MINE')      // buffer untouched
    expect(atBanner.conflict).toEqual({ diskContent: 'THEIRS', diskHash: 'hDisk' })

    keepMine('/w/a.md')
    const afterKeep = snap('/w/a.md')
    expect(afterKeep.currentText).toBe('v1MINE')     // still untouched
    expect(afterKeep.expectedHash).toBe('hDisk')     // CAS baseline adopted
    expect(afterKeep.conflict).toBeNull()
    expect(invokeMock).not.toHaveBeenCalled()        // no immediate write

    // The next autosave tick is what commits the decision.
    edit('/w/a.md', '!')
    await vi.advanceTimersByTimeAsync(1500)
    expect(invokeMock).toHaveBeenCalledWith('document_write', {
      path: '/w/a.md', content: 'v1MINE!', expectedHash: 'hDisk',
    })
  })
})
