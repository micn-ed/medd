// Browser harness stand-in for @tauri-apps/api/event.
//
// Aliasing @tauri-apps/api/core (tauriMock.ts) alone doesn't cover this: @tauri-apps/api/event
// imports invoke via a relative path (./core.js) internal to the package, not the
// '@tauri-apps/api/core' specifier consumer code uses — so a package-specifier alias never
// intercepts it, and listen() would still reach for the real Tauri runtime and throw. Aliasing
// this whole module instead sidesteps that.
//
// WHY THIS RETAINS HANDLERS. It used to discard them and return a no-op unlisten, on the stated
// grounds that "there is no real backend here, so nothing will ever emit
// `document:changed-on-disk`". The premise is true and the conclusion does not follow: **firing an
// event into the frontend's own listeners needs no backend at all** — only somewhere to keep them
// and a way to call them. That mis-attribution mattered, because a platform limitation gets
// accepted and an omission gets fixed, and this one had been accepted for five increments.
//
// What it was hiding: D-11's entire user-facing surface had never been looked at. The conflict
// banner was asserted only in jsdom, which lays nothing out, and the detached state's *absence* of
// UI was discovered by reading App.svelte rather than by seeing it. The harness exists precisely
// because the frontend was unverifiable by eye for six increments — and the one decision where
// autosave can destroy a user's work was still inside that blind spot.
//
// WHAT IT STILL CANNOT TELL YOU, unchanged and worth keeping in front of anyone using it:
//   - It is Blink; the app is WKWebView. This proves appearance and layout, nothing
//     engine-specific — see tauriMock.ts's header for the same caveat applied elsewhere.
//   - It says nothing about whether Rust ever *emits* these events, or with what payload. It
//     exercises what the frontend does on receipt. The emitting side is pinned separately, by
//     watcher.rs's `decide` tests and the `wire_format` pins.
//
// Nothing here is reachable from a production build: vite.config.ts aliases this module only under
// `--mode harness`.

type Handler = (event: { payload: unknown }) => void

const handlers = new Map<string, Set<Handler>>()

export async function listen<T>(
  event: string,
  handler: (event: { payload: T }) => void,
): Promise<() => void> {
  const set = handlers.get(event) ?? new Set()
  set.add(handler as Handler)
  handlers.set(event, set)
  return () => set.delete(handler as Handler)
}

/** Fire an event at whatever is listening, exactly as Rust would. */
function emit(event: string, payload: unknown): void {
  const set = handlers.get(event)
  if (!set || set.size === 0) {
    // eslint-disable-next-line no-console
    console.warn(`harness: nothing is listening for "${event}"`)
    return
  }
  for (const handler of set) handler({ payload })
}

// Convenience triggers for the two states that had no other way to be seen. The payload shapes
// are Rust's, not invented here — see architecture.md §4 and the `wire_format` pins.
const harness = {
  emit,
  /** D-11's dirty branch: type something first, then call this. A clean buffer reloads silently. */
  externalChange(path: string, content = 'changed by someone else\n', hash = 'harness-external') {
    emit('document:changed-on-disk', { path, content, hash })
  },
  /** D-11's deletion branch: the tab stays open holding its text, marked detached. */
  removeOnDisk(path: string) {
    emit('document:removed-on-disk', { path })
  },
  /** Everything currently listening, so you can tell a silent trigger from a wrong event name. */
  listening: () => [...handlers.keys()],
}

declare global {
  interface Window {
    medd: typeof harness
  }
}

if (typeof window !== 'undefined') {
  window.medd = harness
  // eslint-disable-next-line no-console
  console.info(
    'harness: window.medd.externalChange(path) / .removeOnDisk(path) / .emit(event, payload).\n' +
      'For the conflict banner: open a document, type into it, then call externalChange with its path.',
  )
}
