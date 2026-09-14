import { EditorView } from '@codemirror/view'
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language'
import { tags } from '@lezer/highlight'

// Chrome (background, cursor, selection) reads via CSS custom properties defined in app.css,
// so light/dark just falls out of `prefers-color-scheme` — no JS-side theme switching needed.
// This is deliberately modest: a theme that reads in both modes, not the designed typography
// pass that increment 6 owns for the preview pane.
const chrome = EditorView.theme({
  '&': {
    color: 'var(--editor-fg)',
    backgroundColor: 'var(--editor-bg)',
    height: '100%',
    fontSize: '14px',
  },
  '.cm-content': {
    caretColor: 'var(--editor-cursor)',
    fontFamily: 'ui-monospace, monospace',
    padding: '0.75rem 0',
  },
  '.cm-cursor, .cm-dropCursor': {
    borderLeftColor: 'var(--editor-cursor)',
  },
  '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection': {
    backgroundColor: 'var(--editor-selection)',
  },
  '.cm-scroller': {
    overflow: 'auto',
  },
  '&.cm-editor': {
    height: '100%',
  },
})

const highlightStyle = HighlightStyle.define([
  { tag: [tags.heading1, tags.heading2, tags.heading3, tags.heading4, tags.heading5, tags.heading6],
    color: 'var(--editor-heading)',
    fontWeight: 'bold' },
  { tag: tags.strong, fontWeight: 'bold' },
  { tag: tags.emphasis, fontStyle: 'italic' },
  { tag: tags.strikethrough, textDecoration: 'line-through' },
  { tag: [tags.link, tags.url], color: 'var(--editor-link)', textDecoration: 'underline' },
  { tag: tags.monospace, color: 'var(--editor-code-fg)', backgroundColor: 'var(--editor-code-bg)' },
  { tag: tags.quote, color: 'var(--editor-quote)', fontStyle: 'italic' },
  // Markup characters themselves (#, *, _, backticks, list bullets) — kept visible but quiet,
  // so the syntax reads without shouting over the content it's marking up.
  { tag: [tags.processingInstruction, tags.contentSeparator, tags.list], color: 'var(--editor-markup)' },
  { tag: tags.comment, color: 'var(--editor-comment)' },
])

export const markdownTheme = [chrome, syntaxHighlighting(highlightStyle)]
