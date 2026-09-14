import { keymap } from '@codemirror/view'
import { defaultKeymap, historyKeymap, indentWithTab } from '@codemirror/commands'
import { searchKeymap } from '@codemirror/search'

// Reviewed against @codemirror/commands' own source before adding anything: defaultKeymap and
// historyKeymap already carry macOS-conditional bindings (Cmd-ArrowLeft/Right for line
// boundaries, Option-ArrowLeft/Right for word groups, Cmd-ArrowUp/Down for document start/end,
// Cmd-Backspace/Option-Backspace for line/word deletion, Cmd-Z / Cmd-Shift-Z for undo/redo,
// Cmd-A for select all) that already match real macOS text-editing conventions. No gap found
// worth a targeted override yet — this list is the plan's "defaultKeymap plus targeted
// overrides" with zero overrides added speculatively; a real one goes here if the manual pass
// in the real WebView finds one.
//
// indentWithTab is opt-in in CM6 (it makes Tab stop moving focus out of the editor, a real
// accessibility trade) — included because a Markdown source pane without Tab-to-indent for
// lists would be a worse default for this app's actual editing use than the accessibility cost.
export const editorKeymap = keymap.of([
  indentWithTab,
  ...defaultKeymap,
  ...historyKeymap,
  ...searchKeymap,
])
