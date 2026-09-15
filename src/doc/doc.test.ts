// Autosave debounce and D-11 conflict state machine tests (plan-v0.1.md increment 7) — "this
// state machine is small, stateful, and the most dangerous code in the frontend", so exhaustive
// under fake timers rather than sampled. Uses a real EditorView for every edit (not a bare
// state.update()) for the same reason tabs.test.ts does: only a view's dispatch cycle fires the
// update listener that keeps currentText and the retained state honest.
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import { EditorView } from '@codemirror/view'
import { closeAllTabs, editorStateFor, getTab, openTab } from '../tabs'

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

// Imported after the mocks above so doc.ts's own `invoke`/`listen` imports resolve to them.
const { initDocSync, reload, keepMine } = await import('./doc')

function fireBackendEvent(name: string, payload: unknown) {
  listeners.get(name)?.({ payload })
}

function edit(path: string, insert: string) {
  const view = new EditorView({ state: editorStateFor(path) })
  view.dispatch({ changes: { from: view.state.doc.length, insert } })
  view.destroy()
}

beforeEach(() => {
  vi.useFakeTimers()
  closeAllTabs()
  invokeMock.mockReset()
  listeners.clear()
  initDocSync()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('autosave debounce (~1s after typing stops)', () => {
  test('does not fire while still within the debounce window', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    edit('/workspace/a.md', 'X')

    await vi.advanceTimersByTimeAsync(500)

    expect(invokeMock).not.toHaveBeenCalled()
  })

  test('a second edit resets the timer rather than adding a second one', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    edit('/workspace/a.md', 'X')

    await vi.advanceTimersByTimeAsync(700)
    edit('/workspace/a.md', 'Y') // resets the 1s clock

    await vi.advanceTimersByTimeAsync(700)
    expect(invokeMock).not.toHaveBeenCalled() // only 700ms since the second edit

    await vi.advanceTimersByTimeAsync(400)
    expect(invokeMock).toHaveBeenCalledTimes(1)
  })

  test('fires with the current text and the tab expected hash once typing stops', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    invokeMock.mockResolvedValue('h2')
    edit('/workspace/a.md', 'X')

    await vi.advanceTimersByTimeAsync(1000)

    expect(invokeMock).toHaveBeenCalledWith('document_write', {
      path: '/workspace/a.md',
      content: 'v1X',
      expectedHash: 'h1',
    })
  })

  test('a successful write updates lastSyncedText and expectedHash', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    invokeMock.mockResolvedValue('h2')
    edit('/workspace/a.md', 'X')

    await vi.advanceTimersByTimeAsync(1000)

    const tab = getTab('/workspace/a.md')
    expect(tab?.lastSyncedText).toBe('v1X')
    expect(tab?.expectedHash).toBe('h2')
  })

  test('does not fire when the buffer is not dirty', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    // No edit — currentText === lastSyncedText already.
    await vi.advanceTimersByTimeAsync(1500)
    expect(invokeMock).not.toHaveBeenCalled()
  })
})

describe('external change: clean buffer reloads silently (D-11)', () => {
  test('replaces content and adopts the new hash, with no conflict raised', () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    // Not dirty: currentText still equals lastSyncedText.

    fireBackendEvent('document:changed-on-disk', {
      path: '/workspace/a.md',
      content: 'v2 from elsewhere',
      hash: 'h2',
    })

    const tab = getTab('/workspace/a.md')
    expect(tab?.currentText).toBe('v2 from elsewhere')
    expect(tab?.lastSyncedText).toBe('v2 from elsewhere')
    expect(tab?.expectedHash).toBe('h2')
    expect(tab?.conflict).toBeNull()
  })

  test('preserves text outside the changed region rather than replacing the whole document', () => {
    openTab('/workspace/a.md', 'line one\nline two\nline three', 'h1', '/workspace')

    fireBackendEvent('document:changed-on-disk', {
      path: '/workspace/a.md',
      content: 'line one\nline TWO\nline three',
      hash: 'h2',
    })

    expect(getTab('/workspace/a.md')?.currentText).toBe('line one\nline TWO\nline three')
  })

  test('an event for a tab that is no longer open is ignored, not an error', () => {
    expect(() =>
      fireBackendEvent('document:changed-on-disk', {
        path: '/workspace/not-open.md',
        content: 'x',
        hash: 'h',
      }),
    ).not.toThrow()
  })
})

describe('external change: dirty buffer shows the conflict banner (D-11)', () => {
  test('a dirty buffer does not reload — it suspends and records the conflict instead', () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    edit('/workspace/a.md', 'X') // now dirty

    fireBackendEvent('document:changed-on-disk', {
      path: '/workspace/a.md',
      content: 'v2 from elsewhere',
      hash: 'h2',
    })

    const tab = getTab('/workspace/a.md')
    expect(tab?.currentText).toBe('v1X') // untouched
    expect(tab?.conflict).toEqual({ diskContent: 'v2 from elsewhere', diskHash: 'h2' })
  })

  test('autosave is suspended while a conflict is pending', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    edit('/workspace/a.md', 'X')

    fireBackendEvent('document:changed-on-disk', {
      path: '/workspace/a.md',
      content: 'v2',
      hash: 'h2',
    })

    await vi.advanceTimersByTimeAsync(2000)
    expect(invokeMock).not.toHaveBeenCalled()
  })
})

describe('Reload', () => {
  test('takes the disk content, discards the local edit, and clears the conflict', () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    edit('/workspace/a.md', 'X')
    fireBackendEvent('document:changed-on-disk', {
      path: '/workspace/a.md',
      content: 'v2 from elsewhere',
      hash: 'h2',
    })

    reload('/workspace/a.md')

    const tab = getTab('/workspace/a.md')
    expect(tab?.currentText).toBe('v2 from elsewhere')
    expect(tab?.lastSyncedText).toBe('v2 from elsewhere')
    expect(tab?.expectedHash).toBe('h2')
    expect(tab?.conflict).toBeNull()
  })

  test('autosave resumes normally afterwards', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    edit('/workspace/a.md', 'X')
    fireBackendEvent('document:changed-on-disk', {
      path: '/workspace/a.md',
      content: 'v2',
      hash: 'h2',
    })
    reload('/workspace/a.md')

    invokeMock.mockResolvedValue('h3')
    edit('/workspace/a.md', 'Y')
    await vi.advanceTimersByTimeAsync(1000)

    expect(invokeMock).toHaveBeenCalledWith('document_write', {
      path: '/workspace/a.md',
      content: 'v2Y',
      expectedHash: 'h2',
    })
  })
})

describe('Keep mine', () => {
  test('adopts the new hash but does not touch the buffer, and does not write immediately', () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    edit('/workspace/a.md', 'X')
    fireBackendEvent('document:changed-on-disk', {
      path: '/workspace/a.md',
      content: 'v2 from elsewhere',
      hash: 'h2',
    })

    keepMine('/workspace/a.md')

    const tab = getTab('/workspace/a.md')
    expect(tab?.currentText).toBe('v1X') // the user's edit, untouched
    expect(tab?.lastSyncedText).toBe('v2 from elsewhere') // disk's content, not the user's
    expect(tab?.expectedHash).toBe('h2')
    expect(tab?.conflict).toBeNull()
    expect(invokeMock).not.toHaveBeenCalled() // no immediate write
  })

  test('the debounce already pending from the edit that caused the conflict still fires normally, using the new hash', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    edit('/workspace/a.md', 'X') // starts the ~1s debounce
    fireBackendEvent('document:changed-on-disk', {
      path: '/workspace/a.md',
      content: 'v2',
      hash: 'h2',
    })
    keepMine('/workspace/a.md') // does not touch the pending timer, only the CAS baseline

    invokeMock.mockResolvedValue('h3')
    await vi.advanceTimersByTimeAsync(1000) // the timer from the edit above fires

    // "The user's next keystroke is what commits the decision" (architecture.md §3) falls out
    // of dirty being derived, not from any special-cased immediate write in keepMine() itself —
    // this is that same mechanism, just via the timer that was *already* running rather than a
    // brand new keystroke, which is equally valid and arguably the more common real case.
    expect(invokeMock).toHaveBeenCalledWith('document_write', {
      path: '/workspace/a.md',
      content: 'v1X',
      expectedHash: 'h2',
    })
  })
})

describe('a rejected compare-and-swap write becomes a conflict', () => {
  test('document_write rejecting with Conflict raises the same banner as a watcher event', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    invokeMock.mockRejectedValue({
      kind: 'Conflict',
      currentContent: 'v2 raced in',
      hash: 'h-raced',
    })
    edit('/workspace/a.md', 'X')

    await vi.advanceTimersByTimeAsync(1000)

    const tab = getTab('/workspace/a.md')
    expect(tab?.conflict).toEqual({ diskContent: 'v2 raced in', diskHash: 'h-raced' })
    expect(tab?.currentText).toBe('v1X') // the write's rejection never touched the buffer
  })
})

describe('external removal (D-11)', () => {
  test('marks the tab detached and suspends autosave, without recreating the file', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    edit('/workspace/a.md', 'X')

    fireBackendEvent('document:removed-on-disk', { path: '/workspace/a.md' })

    expect(getTab('/workspace/a.md')?.detached).toBe(true)

    await vi.advanceTimersByTimeAsync(2000)
    expect(invokeMock).not.toHaveBeenCalled()
  })
})
