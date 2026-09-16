// Browser harness: a stand-in for @tauri-apps/api/core so the frontend runs in an ordinary
// browser tab, where it can be looked at and driven with normal web tooling.
//
// WHY THIS EXISTS. medd is a Tauri app, and Tauri on macOS has no WebDriver — tauri-driver
// supports Linux and Windows only. That left the entire frontend unverifiable by eye for six
// increments, during which a CSS specificity bug shipped and sat undetected precisely because
// nobody could see it. This makes the visual half of the app inspectable.
//
// WHAT IT CANNOT TELL YOU. The browser is Blink; the app is WKWebView. Anything engine-specific
// — clipboard behaviour, IME composition, native key handling — is *not* covered here and a
// green result in the harness means nothing about it. Those belong to the manual pass in
// plan-v0.1.md increment 12.
//
// Activated only by `npm run harness` (vite --mode harness), which aliases '@tauri-apps/api/core'
// to this file (and '@tauri-apps/api/event' to eventMock.ts, for the same reason but a different
// import path — see that file's header). Nothing here is reachable from a production build.

// THE SHAPES BELOW ARE A COPY OF A CONTRACT THIS FILE DOES NOT OWN. Every payload here is
// hand-constructed to match what Rust serialises — `TreeEntry`/`EntryKind`, `ReadResult`,
// `WorkspaceInfo`, `QuickOpenEntry`, the watcher payloads, `MeddError`. Nothing in the frontend
// checks them against the real thing, and nothing can: a mock at a boundary *defines* that
// boundary for every test that uses it, so agreement here is agreement with itself.
//
// That is not hypothetical. `{kind: "conflict", current_content}` shipped against six green tests
// all mocking `{kind: "Conflict", currentContent}`, and every compare-and-swap rejection missed
// `isConflictError` in production — increment 7's own named blocker case, dead, with the suite
// green. The number of tests over a mocked boundary measures exposure, not coverage.
//
// The contract is owned by the `wire_format` test modules in `src-tauri/src/` — error.rs,
// workspace.rs, quickopen.rs, commands.rs, watcher.rs. They assert the exact bytes Rust emits
// against what the frontend reads. If you change a shape here, change it there, and let that test
// fail first: it is the only place the disagreement is visible.

import { DIRS, FILES, ROOT } from './fixture'

function hashOf(content: string): string {
  // Not a real hash. The harness never writes, so nothing compares these for equality against
  // disk — it exists only to satisfy the shape the frontend expects.
  return `harness-${content.length}`
}

function basename(path: string): string {
  return path.slice(path.lastIndexOf('/') + 1)
}

function kindOf(path: string): 'directory' | 'markdown' | 'other' {
  if (DIRS[path]) return 'directory'
  return path.toLowerCase().endsWith('.md') ? 'markdown' : 'other'
}

export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  switch (cmd) {
    case 'workspace_pick':
      return ROOT as unknown as T

    case 'workspace_open':
      return { root: ROOT, name: basename(ROOT) } as unknown as T

    case 'dir_list': {
      const dir = String(args?.path ?? ROOT)
      const children = DIRS[dir]
      if (!children) throw new Error(`harness: not a directory: ${dir}`)
      return children.map((path) => ({
        name: basename(path),
        path,
        kind: kindOf(path),
      })) as unknown as T
    }

    case 'document_read': {
      const path = String(args?.path)
      const file = FILES[path]
      if (!file) throw new Error(`harness: no such file: ${path}`)
      return { content: file.content, hash: hashOf(file.content) } as unknown as T
    }

    case 'document_write': {
      const path = String(args?.path)
      const file = FILES[path]
      if (!file) throw new Error(`harness: no such file: ${path}`)
      // No real compare-and-swap here: the harness always accepts the write and updates the
      // in-memory fixture, so autosave is visually demonstrable (edit, wait, the content is now
      // "on disk" for the session). There is no foreign writer that could ever produce a genuine
      // conflict in the harness — that path is exercised by the Rust and fake-timer suites, not
      // by looking at this.
      file.content = String(args?.content)
      return hashOf(file.content) as unknown as T
    }

    case 'quick_open_files':
      // Every markdown path in the fixture, flattened — the real command walks the whole
      // workspace tree (deliberately the one place in the app that does), so a flat Object.keys
      // scan over the fixture is a faithful enough stand-in; there's no lazy-cache behaviour here
      // worth modelling since the harness has no watcher to invalidate it.
      return Object.keys(FILES)
        .filter((path) => path.toLowerCase().endsWith('.md'))
        .map((path) => ({
          path,
          relativePath: path.slice(ROOT.length + 1),
        })) as unknown as T

    case 'open_external':
      // eslint-disable-next-line no-console
      console.log('[harness] open_external', args?.url)
      return undefined as unknown as T

    default:
      throw new Error(`harness: unmocked command "${cmd}"`)
  }
}

export function convertFileSrc(path: string): string {
  // Real builds hand this to Tauri's asset protocol. In the harness there is no asset protocol,
  // so a broken-image box is the honest outcome for a path that isn't served — which is also
  // what the real app shows for an image it cannot load.
  return `/harness-asset${path}`
}
