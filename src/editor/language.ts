// @codemirror/lang-markdown's own `markdown()` convenience function defaults to an HTML-aware
// configuration that pulls in @codemirror/lang-html — which unconditionally pulls in the full
// @codemirror/lang-css and @codemirror/lang-javascript grammars too, for a feature (syntax-
// coloured raw HTML blocks embedded in a document) this app's source pane doesn't need: the
// preview pipeline (increment 5) renders HTML separately via markdown-it, never through CM6.
// Using the package's lower-level `markdownLanguage` + `markdownKeymap` exports instead gets
// GFM-aware parsing (tables, strikethrough, task lists) and Enter-continues-list-markup, without
// pulling in any HTML/CSS/JS grammar — confirmed by measuring the built bundle before and after
// this change (see the increment-4 report). Raw HTML in a document still displays and edits
// fine in the source pane; it just isn't specially highlighted.
import { Prec } from '@codemirror/state'
import { keymap } from '@codemirror/view'
import { LanguageSupport } from '@codemirror/language'
import { markdownLanguage, markdownKeymap, pasteURLAsLink } from '@codemirror/lang-markdown'

// pasteURLAsLink (turning a pasted URL over a selection into a markdown link) is independent of
// the html()/css()/javascript() cascade above — a plain DOM paste handler — so it's included at
// no bundle cost, unlike markdownKeymap's sibling completeHTMLTags option, which isn't.
export const markdownSupport = new LanguageSupport(markdownLanguage, [
  Prec.high(keymap.of(markdownKeymap)),
  pasteURLAsLink,
])
