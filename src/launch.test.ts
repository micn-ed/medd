// launch.ts tests (plan-v0.1.md increment 10). QA's own framing for this module: the natural test
// -- attach a listener, emit an open, assert the tab opened -- passes even with no ordering
// discipline at all, because the listener would already be there. The tests below are written in
// the order that actually distinguishes "listens before draining" from "drains before listening".
import { beforeEach, describe, expect, test, vi } from 'vitest'
import type { RouteTarget } from './launch'

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

const { applyRouteTarget, initLaunchRouting } = await import('./launch')

function fire(name: string, payload: unknown) {
  listeners.get(name)?.({ payload })
}

function makeCallbacks() {
  return {
    openWorkspace: vi.fn(async () => {}),
    openFile: vi.fn(async () => {}),
    hasWorkspace: vi.fn(() => false),
    reportErrors: vi.fn(),
  }
}

beforeEach(() => {
  invokeMock.mockReset()
  listeners.clear()
})

describe('applyRouteTarget', () => {
  test('a workspace target opens the workspace and does not touch openFile', async () => {
    const cb = makeCallbacks()
    await applyRouteTarget({ kind: 'workspace', path: '/ws' }, cb)

    expect(cb.openWorkspace).toHaveBeenCalledWith('/ws')
    expect(cb.openFile).not.toHaveBeenCalled()
  })

  test('a document target with a workspace already open just opens the file', async () => {
    const cb = makeCallbacks()
    cb.hasWorkspace.mockReturnValue(true)

    await applyRouteTarget({ kind: 'document', path: '/ws/a.md' }, cb)

    expect(cb.openWorkspace).not.toHaveBeenCalled()
    expect(cb.openFile).toHaveBeenCalledWith('/ws/a.md')
  })

  test('a document target with no workspace yet opens its parent directory first (architecture.md §5)', async () => {
    const cb = makeCallbacks()
    cb.hasWorkspace.mockReturnValue(false)

    await applyRouteTarget({ kind: 'document', path: '/notes/a.md' }, cb)

    expect(cb.openWorkspace).toHaveBeenCalledWith('/notes')
    expect(cb.openFile).toHaveBeenCalledWith('/notes/a.md')
  })
})

describe('initLaunchRouting', () => {
  test('listens before draining -- attaching the open:request listener does not itself depend on frontend_ready resolving', async () => {
    let resolveReady: (targets: RouteTarget[]) => void = () => {}
    invokeMock.mockReturnValue(
      new Promise<RouteTarget[]>((resolve) => {
        resolveReady = resolve
      }),
    )
    const cb = makeCallbacks()

    const initDone = initLaunchRouting(cb)

    // The listener must already be attached here -- before frontend_ready has resolved at all --
    // for a live open arriving in this exact window to have anywhere to land. Firing it now and
    // seeing it handled is the discriminating assertion; a version that attached the listener
    // only after frontend_ready settled would leave `listeners` empty at this point and this
    // event would silently vanish.
    fire('open:request', [{ kind: 'document', path: '/live.md' }])
    await Promise.resolve()
    expect(cb.openFile).toHaveBeenCalledWith('/live.md')

    resolveReady([])
    await initDone
  })

  test('drains whatever frontend_ready returns, in order', async () => {
    invokeMock.mockResolvedValue([
      { kind: 'workspace', path: '/ws' },
      { kind: 'document', path: '/ws/a.md' },
    ] satisfies RouteTarget[])
    const cb = makeCallbacks()

    await initLaunchRouting(cb)

    expect(cb.openWorkspace).toHaveBeenCalledWith('/ws')
    expect(cb.openFile).toHaveBeenCalledWith('/ws/a.md')
  })

  test('a live open:request after frontend_ready resolves opens the forwarded file', async () => {
    invokeMock.mockResolvedValue([])
    const cb = makeCallbacks()
    await initLaunchRouting(cb)

    fire('open:request', [{ kind: 'document', path: '/second-launch.md' }])
    await Promise.resolve()

    expect(cb.openFile).toHaveBeenCalledWith('/second-launch.md')
  })

  test('open:error forwards the raw payload to reportErrors for formatError to render', async () => {
    invokeMock.mockResolvedValue([])
    const cb = makeCallbacks()
    await initLaunchRouting(cb)

    const payload = [{ kind: 'Io', path: '/missing.md', message: 'No such file or directory' }]
    fire('open:error', payload)

    expect(cb.reportErrors).toHaveBeenCalledWith(payload)
  })
})
