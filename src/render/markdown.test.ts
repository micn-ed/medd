// Golden-file tests for the render pipeline (plan-v0.1.md §5) — the regression net for R-1…R-7
// and what makes ADR-001's named comrak escape hatch a measurable change rather than a leap of
// faith. Assertions check for expected HTML fragments rather than whole-document equality: the
// exact wrapping whitespace markdown-it produces isn't the thing under test.
import { describe, expect, test } from 'vitest'
import { renderMarkdown } from './markdown'
import type { RenderContext } from './linkClassification'

const workspace: RenderContext = { documentDir: '/workspace/notes', workspaceRoot: '/workspace' }
const loose: RenderContext = { documentDir: '/elsewhere/docs', workspaceRoot: '/workspace' }
const noWorkspace: RenderContext = { documentDir: '/elsewhere/docs', workspaceRoot: null }

describe('GFM tables (R-1)', () => {
  test('renders a table with a header row', () => {
    const html = renderMarkdown('| a | b |\n|---|---|\n| 1 | 2 |\n', workspace)
    expect(html).toContain('<table>')
    expect(html).toContain('<th>a</th>')
    expect(html).toContain('<td>1</td>')
  })
})

describe('task lists (R-5)', () => {
  test('renders unchecked and checked items as disabled checkboxes', () => {
    const html = renderMarkdown('- [ ] todo\n- [x] done\n', workspace)
    expect(html).toContain('class="task-list-item"')
    expect(html).toMatch(/<input type="checkbox" disabled(?:="")?>\s*todo/)
    expect(html).toMatch(/<input type="checkbox" disabled(?:="")? checked(?:="")?>\s*done/)
  })

  test('leaves an ordinary list item alone', () => {
    const html = renderMarkdown('- just a list item\n', workspace)
    expect(html).not.toContain('checkbox')
    expect(html).toContain('just a list item')
  })
})

describe('footnotes (R-5)', () => {
  test('renders a footnote reference and its definition', () => {
    const html = renderMarkdown('Text.[^1]\n\n[^1]: The note.\n', workspace)
    expect(html).toContain('class="footnote-ref"')
    expect(html).toContain('The note.')
    expect(html).toContain('class="footnote-backref"')
  })
})

describe('strikethrough (R-5)', () => {
  test('renders ~~text~~ as struck through', () => {
    const html = renderMarkdown('~~gone~~\n', workspace)
    expect(html).toContain('<s>gone</s>')
  })
})

describe('fenced code (R-3)', () => {
  test('highlights a curated language', () => {
    const html = renderMarkdown('```rust\nfn main() {}\n```\n', workspace)
    expect(html).toContain('class="hljs-keyword"')
    expect(html).toContain('language-rust')
  })

  test('falls back to escaped, unhighlighted text for an uncurated language', () => {
    const html = renderMarkdown('```brainfuck\n+++.\n```\n', workspace)
    expect(html).not.toContain('hljs-')
    expect(html).toContain('+++.')
  })

  test('resolves toml through the ini alias', () => {
    const html = renderMarkdown('```toml\n[section]\nkey = 1\n```\n', workspace)
    expect(html).toContain('hljs-section')
  })
})

describe('images resolved against the document directory (R-4, D-15)', () => {
  test('a relative image resolves against documentDir, not the workspace root', () => {
    const html = renderMarkdown('![alt](assets/pic.png)\n', workspace)
    expect(html).toContain(
      `src="asset://localhost/${encodeURIComponent('/workspace/notes/assets/pic.png')}"`,
    )
  })

  test('a loose document\'s image resolves against its own directory', () => {
    const html = renderMarkdown('![alt](pic.png)\n', loose)
    expect(html).toContain(
      `src="asset://localhost/${encodeURIComponent('/elsewhere/docs/pic.png')}"`,
    )
  })

  test('an http(s) image source is left untouched (no asset: rewrite)', () => {
    const html = renderMarkdown('![alt](https://example.com/pic.png)\n', workspace)
    expect(html).toContain('src="https://example.com/pic.png"')
  })
})

describe('nested emphasis', () => {
  test('bold containing italic', () => {
    const html = renderMarkdown('**bold *and italic* still bold**\n', workspace)
    expect(html).toContain('<strong>bold <em>and italic</em> still bold</strong>')
  })

  test('italic containing bold', () => {
    const html = renderMarkdown('*italic **and bold** still italic*\n', workspace)
    expect(html).toContain('<em>italic <strong>and bold</strong> still italic</em>')
  })
})

describe('link classification — the four kinds (architecture.md §11)', () => {
  test('external: http(s)', () => {
    const html = renderMarkdown('[ex](https://example.com)\n', workspace)
    expect(html).toContain('data-link-kind="external"')
    expect(html).toContain('href="https://example.com"')
  })

  test('workspace: relative .md resolving inside the open workspace', () => {
    const html = renderMarkdown('[other](other.md)\n', workspace)
    expect(html).toContain('data-link-kind="workspace"')
    expect(html).toContain('href="/workspace/notes/other.md"')
  })

  test('loose: relative .md resolving outside the open workspace (D-15)', () => {
    const html = renderMarkdown('[other](other.md)\n', loose)
    expect(html).toContain('data-link-kind="loose"')
    expect(html).toContain('href="/elsewhere/docs/other.md"')
  })

  test('loose: relative .md when no workspace is open at all', () => {
    const html = renderMarkdown('[other](other.md)\n', noWorkspace)
    expect(html).toContain('data-link-kind="loose"')
  })

  test('anchor: in-document fragment', () => {
    const html = renderMarkdown('[jump](#section)\n', workspace)
    expect(html).toContain('data-link-kind="anchor"')
    expect(html).toContain('href="#section"')
  })

  test('a .md link escaping the workspace via .. is still correctly classified loose', () => {
    const html = renderMarkdown('[out](../../etc/passwd.md)\n', workspace)
    expect(html).toContain('data-link-kind="loose"')
  })

  test('unsupported: relative link to a non-.md file is inert, not silently opened', () => {
    const html = renderMarkdown('[report](report.pdf)\n', workspace)
    expect(html).toContain('data-link-kind="unsupported"')
  })

  test('unsupported: a non-http(s) scheme is inert rather than mis-resolved as a path', () => {
    const html = renderMarkdown('[email](mailto:a@b.com)\n', workspace)
    expect(html).toContain('data-link-kind="unsupported"')
  })
})

describe('DOMPurify sanitisation', () => {
  test('strips a script tag pasted into the document', () => {
    const html = renderMarkdown('<script>alert(1)</script>\n\nText.\n', workspace)
    expect(html).not.toContain('<script>')
    expect(html).toContain('Text.')
  })

  test('keeps a plain, harmless raw HTML element', () => {
    const html = renderMarkdown('<div class="note">a note</div>\n', workspace)
    expect(html).toContain('<div class="note">a note</div>')
  })
})
