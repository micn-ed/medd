// Autosave debounce and D-11 conflict state machine tests (plan-v0.1.md increment 7) — "this
// state machine is small, stateful, and the most dangerous code in the frontend", so exhaustive
// under fake timers rather than sampled. Uses a real EditorView for every edit (not a bare
// state.update()) for the same reason tabs.test.ts does: only a view's dispatch cycle fires the
// update listener that keeps currentText and the retained state honest.
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import { EditorView } from '@codemirror/view'
import { closeAllTabs, closeTab, editorStateFor, getTab, openTab } from '../tabs'

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
const { initDocSync, reload, keepMine, flushAll, waitForQuiescence, waitForAllQuiescent } =
  await import('./doc')

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

  test("a write's outcome records what was written, not what the buffer holds when it settles", async () => {
    // performWrite calls markSynced(path, request.content, settledHash) — request.content is
    // captured when the write is ISSUED, at the top of requestWrite. If markSynced were ever
    // changed to read the tab's current buffer instead (mutants.sh's "markSynced records the
    // buffer, not what was written"), lastSyncedText would claim disk holds text that was never
    // actually sent, for any edit landing after issue but before the write settles.
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    let resolveWrite: (hash: string) => void = () => {}
    invokeMock.mockReturnValue(
      new Promise<string>((resolve) => {
        resolveWrite = resolve
      }),
    )
    edit('/workspace/a.md', 'X') // -> 'v1X'
    await vi.advanceTimersByTimeAsync(1000) // the write for 'v1X' is now in flight, unsettled

    edit('/workspace/a.md', 'Y') // -> 'v1XY', typed while that write is still in flight; this
    // schedules its own fresh debounce timer rather than joining the in-flight write, and it's
    // deliberately never advanced to firing — the point is what the FIRST write records, not what
    // happens to a second one.

    resolveWrite!('h2')
    await Promise.resolve()
    await Promise.resolve()

    const tab = getTab('/workspace/a.md')
    expect(tab?.lastSyncedText).toBe('v1X') // what the in-flight write actually sent
    expect(tab?.currentText).toBe('v1XY') // the later edit, still dirty and unsaved
  })

  test('does not fire when the buffer is not dirty', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    // No edit — currentText === lastSyncedText already.
    await vi.advanceTimersByTimeAsync(1500)
    expect(invokeMock).not.toHaveBeenCalled()
  })

  test('a stale pre-conflict timer, left uncancelled by Reload, still finds nothing dirty to write', async () => {
    // The debounce timer an edit schedules is never explicitly cancelled by a conflict arriving
    // or by Reload resolving it — "autosave is suspended while a conflict is pending" (above)
    // shows the write is blocked while conflict is still set, but that only proves the CONFLICT
    // guard in requestWrite works. By the time this test's original timer fires, Reload has
    // already cleared the conflict and made the buffer clean again, so this isolates the other
    // guard: requestWrite's dirty check is what's actually standing between a stale timer and an
    // unwanted write once the conflict guard is no longer the one stopping it.
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    edit('/workspace/a.md', 'X') // schedules the timer this test is about

    fireBackendEvent('document:changed-on-disk', {
      path: '/workspace/a.md',
      content: 'v2 from elsewhere',
      hash: 'h2',
    })
    reload('/workspace/a.md') // clears the conflict; currentText and lastSyncedText both become
    // 'v2 from elsewhere' — clean. The timer scheduled above is still pending and uncancelled.

    await vi.advanceTimersByTimeAsync(1500) // the stale timer fires well within this window

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

  test('a debounce already pending from the edit that caused the conflict still results in exactly one write', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    edit('/workspace/a.md', 'X') // starts the ~1s debounce
    fireBackendEvent('document:changed-on-disk', {
      path: '/workspace/a.md',
      content: 'v2',
      hash: 'h2',
    })
    invokeMock.mockResolvedValue('h3')
    keepMine('/workspace/a.md') // reschedules on top of the still-pending timer from the edit

    await vi.advanceTimersByTimeAsync(1000)

    expect(invokeMock).toHaveBeenCalledTimes(1)
    expect(invokeMock).toHaveBeenCalledWith('document_write', {
      path: '/workspace/a.md',
      content: 'v1X',
      expectedHash: 'h2',
    })
  })

  // increment-7 review, finding 2 (blocker): resolveConflictKeepMine is a pure state transition
  // with no I/O, so whether "Keep mine" ever reached disk used to depend entirely on a debounce
  // timer happening to survive the conflict — and on the plan's own named route (a rejected CAS
  // write), it never does: the timer fired *to perform* that write, so it's spent by the time the
  // conflict exists. keepMine() now schedules unconditionally on resolution so this is no longer
  // a coincidence.
  test('a rejected CAS write spends the pending timer, but Keep mine schedules its own and the write still lands', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    invokeMock.mockRejectedValueOnce({
      kind: 'Conflict',
      currentContent: 'v2 from elsewhere',
      hash: 'h2',
    })
    edit('/workspace/a.md', 'X')

    await vi.advanceTimersByTimeAsync(1000) // the timer fires, the write is rejected as a conflict

    expect(getTab('/workspace/a.md')?.conflict).toEqual({
      diskContent: 'v2 from elsewhere',
      diskHash: 'h2',
    })
    expect(invokeMock).toHaveBeenCalledTimes(1) // the one rejected attempt -- no timer survives it

    invokeMock.mockResolvedValue('h3')
    keepMine('/workspace/a.md')

    await vi.advanceTimersByTimeAsync(1000)

    expect(invokeMock).toHaveBeenCalledTimes(2)
    expect(invokeMock).toHaveBeenLastCalledWith('document_write', {
      path: '/workspace/a.md',
      content: 'v1X',
      expectedHash: 'h2',
    })
  })
})

// QA's acceptance pass on increment 7, finding 1 (blocker): closing a tab used to silently drop
// whatever the debounce hadn't yet written -- `debounceTimers` is keyed by path, `closeTab` never
// touched it, and by the time the timer fired `getTab(path)` returned nothing. "Always safe, never
// prompts" (P-2) was false for exactly the ordinary gesture of fixing a typo and hitting Cmd+W
// inside the one-second window.
describe('closing a tab flushes a pending autosave rather than dropping it (QA finding 1)', () => {
  test('typing then closing within the debounce window still reaches disk', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    invokeMock.mockResolvedValue('h2')
    edit('/workspace/a.md', 'X') // starts the ~1s debounce

    closeTab('/workspace/a.md') // the user closes before the debounce fires

    await vi.advanceTimersByTimeAsync(5000)

    expect(invokeMock).toHaveBeenCalledWith('document_write', {
      path: '/workspace/a.md',
      content: 'v1X',
      expectedHash: 'h1',
    })
  })

  test('closing a clean tab writes nothing -- there is nothing to flush', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    closeTab('/workspace/a.md')

    await vi.advanceTimersByTimeAsync(5000)

    expect(invokeMock).not.toHaveBeenCalled()
  })

  test('closing a tab whose write already completed does not write a second time', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    invokeMock.mockResolvedValue('h2')
    edit('/workspace/a.md', 'X')
    await vi.advanceTimersByTimeAsync(1000) // the debounce fires and completes normally

    closeTab('/workspace/a.md') // nothing pending left to flush

    await vi.advanceTimersByTimeAsync(5000)

    expect(invokeMock).toHaveBeenCalledTimes(1)
  })

  test('closeAllTabs (the D-14 workspace switch) flushes every open tab at once', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    edit('/workspace/a.md', 'X')
    openTab('/workspace/b.md', 'v1', 'h1', '/workspace')
    edit('/workspace/b.md', 'Y')
    invokeMock.mockResolvedValue('hnew')

    closeAllTabs()

    await vi.advanceTimersByTimeAsync(5000)

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
  })

  // A third route the leader's review found: no pending *timer* at close time (so the check
  // above finds nothing to flush), but a second edit arrived while the first write was already in
  // flight, and got captured as a queued follow-up rather than issued directly (see `requestWrite`
  // in doc.ts). The old `pendingRetry` mechanism re-read the tab when that follow-up finally ran,
  // which is exactly what a closed tab can no longer provide -- a real, on-screen edit ("v1AB")
  // vanished. The fix captures the follow-up's content up front, so it needs nothing from the tab
  // by the time it fires.
  test('a write queued behind an in-flight one still reaches disk after the tab closes', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    let resolveFirstWrite: (hash: string) => void = () => {}
    invokeMock.mockImplementationOnce(
      () =>
        new Promise<string>((resolve) => {
          resolveFirstWrite = resolve
        }),
    )
    edit('/workspace/a.md', 'A')
    await vi.advanceTimersByTimeAsync(1000) // tick 1 fires; document_write("v1A") is now in flight

    edit('/workspace/a.md', 'B')
    await vi.advanceTimersByTimeAsync(1000) // tick 2 fires; a write is already in flight -> queued

    expect(invokeMock).toHaveBeenCalledTimes(1) // queued, not sent -- no second call yet

    closeTab('/workspace/a.md') // no pending timer left to flush; the queue already has "v1AB"

    invokeMock.mockResolvedValue('h3') // what the queued write will receive once it fires
    resolveFirstWrite!('h2') // the first write lands
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()

    expect(invokeMock).toHaveBeenCalledTimes(2)
    expect(invokeMock).toHaveBeenLastCalledWith('document_write', {
      path: '/workspace/a.md',
      content: 'v1AB',
      expectedHash: 'h2', // chained onto the first write's real new hash, not the stale one
    })
  })
})

// Increment-7 review finding 3 / QA's integration hazard (blocker D): §3's flowchart has no
// notion of a write being in flight when the tab's state moves underneath it. `performAutosave`
// captures a per-path generation before its `await`; if anything moved the generation while the
// write was airborne, its outcome is dropped rather than applied -- "a write's outcome may only
// be applied to the state that issued it" (architecture.md §3). Deferred promises stand in for
// `document_write` here so a write can be held "in flight" across other calls in the same test.
describe("a write's outcome is dropped if the tab's state moved while it was in flight (review finding 3)", () => {
  test('3a: a write settling after Reload does not clobber the resolved state', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    let resolveWrite: (hash: string) => void = () => {}
    invokeMock.mockReturnValue(
      new Promise<string>((resolve) => {
        resolveWrite = resolve
      }),
    )
    edit('/workspace/a.md', 'X')
    await vi.advanceTimersByTimeAsync(1000) // debounce fires; document_write is now in flight

    fireBackendEvent('document:changed-on-disk', {
      path: '/workspace/a.md',
      content: 'DISK',
      hash: 'hDISK',
    })
    expect(getTab('/workspace/a.md')?.conflict).not.toBeNull() // still dirty -- write hasn't settled

    reload('/workspace/a.md') // resolved before the stale write below settles

    resolveWrite!('hSTALE')
    await Promise.resolve()
    await Promise.resolve()

    const tab = getTab('/workspace/a.md')
    expect(tab?.currentText).toBe('DISK')
    expect(tab?.lastSyncedText).toBe('DISK') // not clobbered by the stale write's own text
    expect(tab?.expectedHash).toBe('hDISK') // not overwritten with the stale write's hash
    expect(tab?.conflict).toBeNull() // still resolved, not resurrected
  })

  test('3a: a stale write rejected as Conflict after Reload does not resurrect the banner', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    let rejectWrite: (err: unknown) => void = () => {}
    invokeMock.mockReturnValue(
      new Promise((_resolve, reject) => {
        rejectWrite = reject
      }),
    )
    edit('/workspace/a.md', 'X')
    await vi.advanceTimersByTimeAsync(1000)

    fireBackendEvent('document:changed-on-disk', {
      path: '/workspace/a.md',
      content: 'DISK',
      hash: 'hDISK',
    })
    reload('/workspace/a.md')

    rejectWrite!({ kind: 'Conflict', currentContent: 'stale conflict', hash: 'hStaleConflict' })
    await Promise.resolve()
    await Promise.resolve()

    // A banner reappearing here would be exactly what D-11's own rationale calls fatal: one
    // firing on a buffer nobody has touched since it was resolved.
    expect(getTab('/workspace/a.md')?.conflict).toBeNull()
  })

  test('3b: closing and reopening during an in-flight write does not clobber the reopened tab', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    let resolveWrite: (hash: string) => void = () => {}
    invokeMock.mockReturnValue(
      new Promise<string>((resolve) => {
        resolveWrite = resolve
      }),
    )
    edit('/workspace/a.md', 'X')
    await vi.advanceTimersByTimeAsync(1000) // debounce fires; write in flight with expectedHash h1

    closeTab('/workspace/a.md') // nothing pending to flush -- the write is already airborne

    // A fresh read from disk, unrelated to the stale in-flight write above.
    openTab('/workspace/a.md', 'FRESH FROM DISK', 'hFresh', '/workspace')

    resolveWrite!('hSTALE')
    await Promise.resolve()
    await Promise.resolve()

    const tab = getTab('/workspace/a.md')
    expect(tab?.currentText).toBe('FRESH FROM DISK')
    expect(tab?.lastSyncedText).toBe('FRESH FROM DISK') // not clobbered by the stale write
    expect(tab?.expectedHash).toBe('hFresh')
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

// The quiescence handle (leader review, following the route-C fix): a caller about to
// `document_read` a path must not do so while a write for it is still airborne, since the read
// can land on disk a moment before that write commits and hand back a baseline the very next
// settle makes stale (App.svelte's `openFile` now awaits this before reading). It doubles as the
// primitive a future quit path needs to know every write has actually landed before letting the
// process exit.
describe('waitForQuiescence', () => {
  test('resolves immediately for a path with nothing outstanding', async () => {
    let resolved = false
    void waitForQuiescence('/workspace/never-touched.md').then(() => {
      resolved = true
    })
    await Promise.resolve()
    expect(resolved).toBe(true)
  })

  test('does not resolve until an in-flight write settles', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    let resolveWrite: (hash: string) => void = () => {}
    invokeMock.mockReturnValue(
      new Promise<string>((resolve) => {
        resolveWrite = resolve
      }),
    )
    edit('/workspace/a.md', 'X')
    await vi.advanceTimersByTimeAsync(1000) // the write is now in flight

    let resolved = false
    void waitForQuiescence('/workspace/a.md').then(() => {
      resolved = true
    })
    await Promise.resolve()
    expect(resolved).toBe(false)

    resolveWrite!('h2')
    await Promise.resolve()
    await Promise.resolve()
    expect(resolved).toBe(true)
  })

  // The subtlety the leader's review named explicitly: quiescence is not monotonic within a
  // drain. `performWrite`'s own `finally` can immediately start a second write for a request
  // queued behind the first, so a naive one-shot check taken right as the first write settles
  // would report quiescent when a second write is, in fact, about to begin.
  test('does not resolve after the first of two chained writes -- only after both settle', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    let resolveFirstWrite: (hash: string) => void = () => {}
    invokeMock.mockImplementationOnce(
      () =>
        new Promise<string>((resolve) => {
          resolveFirstWrite = resolve
        }),
    )
    edit('/workspace/a.md', 'A')
    await vi.advanceTimersByTimeAsync(1000) // tick 1 fires; write "v1A" in flight

    edit('/workspace/a.md', 'B')
    await vi.advanceTimersByTimeAsync(1000) // tick 2 fires; a write is in flight -> "v1AB" queued

    let resolved = false
    void waitForQuiescence('/workspace/a.md').then(() => {
      resolved = true
    })

    let resolveSecondWrite: (hash: string) => void = () => {}
    invokeMock.mockImplementationOnce(
      () =>
        new Promise<string>((resolve) => {
          resolveSecondWrite = resolve
        }),
    )
    resolveFirstWrite!('h2') // the first write lands; the queued one starts immediately
    await Promise.resolve()
    await Promise.resolve()
    expect(resolved).toBe(false) // still not quiescent -- the chained write is now in flight

    resolveSecondWrite!('h3')
    await Promise.resolve()
    await Promise.resolve()
    expect(resolved).toBe(true)
  })

  test('waitForAllQuiescent waits for every outstanding path, not just one', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    openTab('/workspace/b.md', 'v1', 'h1', '/workspace')
    let resolveA: (hash: string) => void = () => {}
    let resolveB: (hash: string) => void = () => {}
    invokeMock.mockImplementation(
      (_cmd: string, args?: { path?: string }) =>
        new Promise<string>((resolve) => {
          if (args?.path === '/workspace/a.md') resolveA = resolve
          else resolveB = resolve
        }),
    )
    edit('/workspace/a.md', 'X')
    edit('/workspace/b.md', 'Y')
    await vi.advanceTimersByTimeAsync(1000) // both writes now in flight

    let resolved = false
    const done = waitForAllQuiescent().then(() => {
      resolved = true
    })

    resolveA!('h2')
    await Promise.resolve()
    await Promise.resolve()
    expect(resolved).toBe(false) // b.md is still outstanding

    resolveB!('h2')
    await done // awaits the real promise chain rather than guessing a microtask-tick count
    expect(resolved).toBe(true)
  })
})

describe('flushAll', () => {
  test('flushes every open tab, not just one path', async () => {
    openTab('/workspace/a.md', 'v1', 'h1', '/workspace')
    edit('/workspace/a.md', 'X')
    openTab('/workspace/b.md', 'v1', 'h1', '/workspace')
    edit('/workspace/b.md', 'Y')
    invokeMock.mockResolvedValue('hnew')

    flushAll()

    await vi.advanceTimersByTimeAsync(0)

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
  })
})
