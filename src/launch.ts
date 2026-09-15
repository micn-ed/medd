// Launch routing, the frontend half (plan-v0.1.md increment 10, ADR-003). Rust's `route_open`
// has already canonicalised and classified every path by the time anything here runs; this
// module's only remaining decision is architecture.md §5's rule that isn't Rust's to make — "if
// there is no workspace yet, a loose file's parent directory becomes the root" — everything else
// is orchestration (listen, invoke, apply), kept out of App.svelte so it can be tested with a
// mocked backend instead of a mounted component.
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { dirname } from './render'

export interface RouteTarget {
  kind: 'workspace' | 'document'
  path: string
}

export interface LaunchCallbacks {
  openWorkspace: (path: string) => Promise<void>
  openFile: (path: string) => Promise<void>
  hasWorkspace: () => boolean
  reportErrors: (errors: unknown[]) => void
}

export async function applyRouteTarget(target: RouteTarget, cb: LaunchCallbacks): Promise<void> {
  if (target.kind === 'workspace') {
    await cb.openWorkspace(target.path)
    return
  }
  // A document target with no workspace yet is the CLI's or Finder's most common shape (`medd
  // notes/todo.md` with nothing open) — its parent directory becomes the root so the sidebar has
  // something to show, rather than leaving the file open with no tree at all.
  if (!cb.hasWorkspace()) await cb.openWorkspace(dirname(target.path))
  await cb.openFile(target.path)
}

/** Wires the two backend launch signals to `cb` and drains whatever arrived before this was
 * called. The listener attaches *before* `frontend_ready` is invoked, deliberately (QA's own
 * criterion for this exact ordering): a live open arriving the instant Rust marks itself ready
 * must already have somewhere to land, not race this function's own await chain to get there. */
export async function initLaunchRouting(cb: LaunchCallbacks): Promise<void> {
  await listen<RouteTarget[]>('open:request', (event) => {
    for (const target of event.payload) void applyRouteTarget(target, cb)
  })
  await listen<unknown[]>('open:error', (event) => {
    cb.reportErrors(event.payload)
  })

  const buffered = await invoke<RouteTarget[]>('frontend_ready')
  for (const target of buffered) await applyRouteTarget(target, cb)
}
