// GFM task lists (`- [ ] foo` / `- [x] foo`), written first-party rather than taking a
// dependency. The two candidate plugins were checked (ADR-001 asks for exactly this before
// wiring up any plugin) and both trace to the same source: `markdown-it-task-lists` hasn't been
// touched since 2022 and carries 8 open issues; `@hackmd/markdown-it-task-lists` is a 2024
// republish of the identical code, not an independently maintained fork — its own package
// metadata still points at the dormant upstream repo. The feature itself is small and the GFM
// syntax hasn't changed, so a ~40-line first-party rule is less risk than either.
// markdown-it's own type declarations export the class *instance* type (`MarkdownIt`) as a
// named type, separately from the default export (a callable constructor value) — importing
// both under the same local name collides, so the instance type is aliased here.
import type { MarkdownIt as MarkdownItInstance, Token } from 'markdown-it'

const TASK_ITEM = /^\[([ xX])\]\s+/

export function taskLists(md: MarkdownItInstance): void {
  md.core.ruler.after('inline', 'task_lists', (state) => {
    const tokens = state.tokens
    for (let i = 0; i < tokens.length; i++) {
      if (tokens[i].type !== 'list_item_open') continue

      const inlineIndex = firstInlineIn(tokens, i)
      if (inlineIndex === -1) continue

      const inline = tokens[inlineIndex]
      const firstChild = inline.children?.[0]
      if (!firstChild || firstChild.type !== 'text') continue

      const match = TASK_ITEM.exec(firstChild.content)
      if (!match) continue

      const checked = match[1].toLowerCase() === 'x'
      firstChild.content = firstChild.content.slice(match[0].length)

      const checkbox = new state.Token('html_inline', '', 0)
      checkbox.content = `<input type="checkbox" disabled${checked ? ' checked' : ''}>`
      inline.children!.unshift(checkbox)

      const existing = tokens[i].attrGet('class')
      tokens[i].attrSet('class', existing ? `${existing} task-list-item` : 'task-list-item')
    }
    return true
  })
}

/** The first `inline` token inside the list item starting at `listItemOpenIndex`, or -1 if the
 * item closes (or the token stream ends) before one is found. */
function firstInlineIn(tokens: Token[], listItemOpenIndex: number): number {
  for (let j = listItemOpenIndex + 1; j < tokens.length; j++) {
    if (tokens[j].type === 'list_item_close') return -1
    if (tokens[j].type === 'inline') return j
  }
  return -1
}
