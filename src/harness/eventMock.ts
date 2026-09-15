// Browser harness stand-in for @tauri-apps/api/event.
//
// Aliasing @tauri-apps/api/core (tauriMock.ts) alone doesn't cover this: @tauri-apps/api/event
// imports invoke via a relative path (./core.js) internal to the package, not the
// '@tauri-apps/api/core' specifier consumer code uses — so a package-specifier alias never
// intercepts it, and listen() would still reach for the real Tauri runtime and throw. Aliasing
// this whole module instead sidesteps that.
//
// There is no real backend here, so nothing will ever emit `document:changed-on-disk` or
// `document:removed-on-disk` — listen() only satisfies the shape doc.ts expects (a promise
// resolving to an unlisten function) and never calls the handler. External-change detection is
// exactly the kind of thing this harness cannot prove; see tauriMock.ts's own header for the
// same caveat applied elsewhere.
export async function listen<T>(
  _event: string,
  _handler: (event: { payload: T }) => void,
): Promise<() => void> {
  return () => {}
}
