// Fixture workspace for the browser harness. Not shipped — see tauriMock.ts.
//
// The content here is chosen to exercise every rendering feature v0.1 claims (R-1…R-7) in one
// screenful each, so a visual pass has something to actually look at rather than lorem.

export interface FixtureFile {
  content: string
}

export const ROOT = '/fixture/workspace'

export const FILES: Record<string, FixtureFile> = {
  [`${ROOT}/README.md`]: {
    content: `# Reading test

A paragraph of ordinary prose, long enough to show where the measure falls and whether the
line length is comfortable to read. The point of this document is to make every rendering
feature visible at once, so typography can be judged rather than guessed at.

Inline treatments: **bold**, *italic*, ***both***, \`inline code\`, ~~struck through~~, and a
[link to another document](architecture.md) that should open as a tab rather than a browser.

## Heading level two

Some following text, so the space above and below a heading can be judged against the body
rhythm rather than in isolation.

### Heading level three

And again at the third level, which is where most documents actually live.

> A blockquote, for when a document is quoting something.
> It runs to a second line so the left border and the inset are both visible.

---

## Tables

| Increment | Name | Status | Bundle (gzip) |
|---|---|---:|---:|
| 1 | Skeleton | done | 14.67 kB |
| 2 | Document core | done | — |
| 4 | Source pane | done | 124.16 kB |
| 5 | Render pipeline | done | 196.28 kB |
| 6 | Typography | done | 197.03 kB |
| 8 | Tabs | done | 198.01 kB |

## Code

\`\`\`rust
/// Compare-and-swap write: a mismatch rejects and touches nothing on disk.
pub fn write(&self, path: &Path, content: &str, expected: &ContentHash) -> Result<ContentHash> {
    let canonical = canonicalize(path)?;
    let mut last_known = self.last_known.lock().unwrap();
    let current = ContentHash::of(&fs::read(&canonical)?);
    if current != *expected {
        return Err(MeddError::Conflict { hash: current });
    }
    atomic_write(&canonical, content.as_bytes())?;
    Ok(ContentHash::of(content.as_bytes()))
}
\`\`\`

\`\`\`bash
make dev      # run in development
make build    # produce the .app bundle
\`\`\`

\`\`\`
A fenced block with no language at all, which should still render as code
without highlighting rather than falling back to a paragraph.
\`\`\`

## Lists

1. An ordered item
2. A second, with a nested list beneath it
   - nested unordered
   - and another
3. A third

- [x] A completed task
- [ ] An outstanding one
- [ ] A third, to show spacing between items

## Footnotes

A claim that needs support.[^1] And a second one, further along.[^2]

[^1]: The supporting note, rendered at the foot of the document.
[^2]: A second note, so the list has more than one entry to space.
`,
  },
  [`${ROOT}/architecture.md`]: {
    content: `# Architecture

A second document, so opening it proves tab switching and per-tab view mode.

The line medd hangs from: **Rust owns the filesystem, the WebView owns the document.** A
keystroke never crosses the IPC bridge.

## Why that matters

| Layer | Runs in |
|---|---|
| Markdown parse | WebView |
| Syntax highlight | WebView |
| Atomic write | Rust |
| File watching | Rust |

Back to [the README](README.md), or out to [an external site](https://example.com).
`,
  },
  [`${ROOT}/notes/scratch.md`]: {
    content: `# Scratch

A document one level down, so the tree has something to expand into.

- a short list
- of short items
`,
  },
  [`${ROOT}/notes/empty.md`]: { content: '' },
  [`${ROOT}/LICENSE`]: { content: 'Not markdown — should be visible but inert.' },
}

export const DIRS: Record<string, string[]> = {
  [ROOT]: [`${ROOT}/notes`, `${ROOT}/README.md`, `${ROOT}/architecture.md`, `${ROOT}/LICENSE`],
  [`${ROOT}/notes`]: [`${ROOT}/notes/scratch.md`, `${ROOT}/notes/empty.md`],
}
