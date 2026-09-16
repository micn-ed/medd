// Carried finding 6: no path may hand the frontend content `read()` would have refused.
//
// The Rust half is in document.rs (both doors) and the wire format is pinned in error.rs. This is
// the frontend's half: what happens to a tab whose document has become something medd cannot
// represent.
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

function edit(path: string, insert: string) {
  const v = new EditorView({ state: editorStateFor(path) })
  v.dispatch({ changes: { from: v.state.doc.length, insert } })
  v.destroy()
}

beforeEach(() => {
  vi.useFakeTimers()
  closeAllTabs()
  invokeMock.mockReset()
  listeners.clear()
  initDocSync()
})
afterEach(() => vi.useRealTimers())

describe('a document that has become non-UTF-8 on disk', () => {
  test('detaches the tab rather than logging into a void', async () => {
    // The payload shape here is the one error.rs's wire_format tests pin. It is deliberately not
    // invented to suit this file: inventing it is how the CAS-conflict path shipped dead.
    openTab('/w/a.md', 'v1', 'h1', '/w')
    invokeMock.mockRejectedValue({ kind: 'NotUtf8', path: '/w/a.md' })
    edit('/w/a.md', 'MINE')

    await vi.advanceTimersByTimeAsync(1000)

    const tab = getTab('/w/a.md')!
    expect(tab.detached).toBe(true)
    // Not a conflict: there is only one version, so a banner offering Reload would offer nothing.
    expect(tab.conflict).toBeNull()
    // The user's text is untouched — it is the good version.
    expect(tab.currentText).toBe('v1MINE')
  })

  test('stops retrying against a file it can never write', async () => {
    openTab('/w/a.md', 'v1', 'h1', '/w')
    invokeMock.mockRejectedValue({ kind: 'NotUtf8', path: '/w/a.md' })
    edit('/w/a.md', 'A')
    await vi.advanceTimersByTimeAsync(1000)
    const afterFirst = invokeMock.mock.calls.length
    expect(afterFirst).toBe(1)

    // `detached` suspends autosave, so further typing must not produce further doomed writes.
    edit('/w/a.md', 'B')
    await vi.advanceTimersByTimeAsync(5000)

    expect(invokeMock.mock.calls.length).toBe(afterFirst)
  })

  test('an Io failure still only logs — detaching is specific to NotUtf8', async () => {
    // The else branch has to stay distinguishable: an Io error during autosave is transient and
    // must not latch a tab into a terminal state.
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {})
    openTab('/w/b.md', 'v1', 'h1', '/w')
    invokeMock.mockRejectedValue({ kind: 'Io', path: '/w/b.md', message: 'transient' })
    edit('/w/b.md', 'X')

    await vi.advanceTimersByTimeAsync(1000)

    expect(getTab('/w/b.md')!.detached).toBe(false)
    expect(spy).toHaveBeenCalled()
    spy.mockRestore()
  })
})
