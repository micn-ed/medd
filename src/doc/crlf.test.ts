// QA: the CRLF guarantee as the FRONTEND experiences it.
//
// The fix lives entirely in Rust — check_external_change and the Conflict payload both emit
// LF-normalised content now. The frontend has no normalisation of its own, so these tests pin
// two separate things: that the contract holds when it is honoured, and what breaks the moment
// it is not. The second is the point: architecture.md §3 names read and write but not
// check_external_change, so the specification is narrower than the fix, and nothing on this
// side would catch a regression on the one path the fix was written for.
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import { EditorView } from '@codemirror/view'
import { closeAllTabs, editorStateFor, getTab, openTab, registerMountedView, unregisterMountedView } from '../tabs'

const { invokeMock, listeners } = vi.hoisted(() => ({
  invokeMock: vi.fn(), listeners: new Map<string, (e: { payload: unknown }) => void>(),
}))
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((e: string, h: (x: { payload: unknown }) => void) => {
    listeners.set(e, h); return Promise.resolve(() => {})
  }),
}))
const doc = await import('./doc')
const fire = (n: string, p: unknown) => listeners.get(n)?.({ payload: p })

beforeEach(() => {
  vi.useFakeTimers(); closeAllTabs(); invokeMock.mockReset()
  invokeMock.mockResolvedValue('hNew'); listeners.clear(); doc.initDocSync()
})
afterEach(() => vi.useRealTimers())

// A CRLF document as the frontend receives it: already LF, because document_read normalised it.
const AS_OPENED = 'alpha\nbravo\ncharlie\n'

describe('a clean external reload of a CRLF document', () => {
  test('leaves the buffer matching disk, clean, with no autosave', async () => {
    const path = '/w/crlf.md'
    openTab(path, AS_OPENED, 'h1', '/w')
    const view = new EditorView({ state: editorStateFor(path) })
    registerMountedView(path, view)

    // The watcher's payload for "someone changed one word in Neovim" — LF, per the Rust fix.
    fire('document:changed-on-disk', { path, content: 'alpha\nBRAVO\ncharlie\n', hash: 'h2' })

    const tab = getTab(path)!
    expect(tab.currentText).toBe('alpha\nBRAVO\ncharlie\n')
    expect(tab.currentText).toBe(tab.lastSyncedText)     // clean
    expect(tab.conflict).toBeNull()
    expect(view.state.doc.toString()).toBe('alpha\nBRAVO\ncharlie\n')
    expect(view.state.doc.lines).toBe(4)                 // 3 lines + trailing newline, no extra

    await vi.advanceTimersByTimeAsync(5000)
    expect(invokeMock).not.toHaveBeenCalled()            // nothing to write — it was not dirty

    unregisterMountedView(path); view.destroy()
  })

  test('no carriage return ever reaches the editor buffer', () => {
    const path = '/w/crlf.md'
    openTab(path, AS_OPENED, 'h1', '/w')
    fire('document:changed-on-disk', { path, content: 'alpha\nBRAVO\ncharlie\n', hash: 'h2' })
    // The absence of \r is also true of a buffer nothing touched, so pin the reload first.
    expect(getTab(path)!.currentText).toBe('alpha\nBRAVO\ncharlie\n')
    expect(getTab(path)!.currentText).not.toContain('\r')
  })
})

describe('what the frontend does if raw CRLF ever reaches it', () => {
  // Not a defect today — Rust normalises. This pins the blast radius, and is the regression
  // test for the seam itself rather than for either side of it.
  test('the buffer goes dirty and autosave writes back CodeMirror-normalised text', async () => {
    const path = '/w/crlf.md'
    openTab(path, AS_OPENED, 'h1', '/w')
    const view = new EditorView({ state: editorStateFor(path) })
    registerMountedView(path, view)

    fire('document:changed-on-disk', { path, content: 'alpha\r\nBRAVO\r\ncharlie\r\n', hash: 'h2' })

    const tab = getTab(path)!
    // CodeMirror strips the \r on the way in, so lastSyncedText (raw, from the payload) and
    // currentText (what the editor holds) disagree — the document reads as dirty without the
    // user having touched it.
    expect(tab.lastSyncedText).toContain('\r')
    expect(tab.currentText).not.toContain('\r')
    expect(tab.currentText === tab.lastSyncedText).toBe(false)

    await vi.advanceTimersByTimeAsync(5000)
    expect(invokeMock).toHaveBeenCalled()   // an unrequested write, of a file nobody edited

    unregisterMountedView(path); view.destroy()
  })
})
