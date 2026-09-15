# medd — MVP Scope

The user's chosen MVP bar is a **thin vertical slice**: open a folder, browse it, edit a file,
see it rendered, and have it saved. Everything else — including two of the three launch paths —
comes after that slice works end to end.

The reasoning behind that choice is worth stating: the launch integrations (Finder, Neovim) are
all thin wrappers around "tell the running app to open this path". They are cheap to add *once
the app can open a path at all*, and expensive to debug against an app that does not yet work.
Building the core first means the integrations land on solid ground.

---

## v0.1 — The thin slice

**Definition of done:** the user can open a folder of Markdown files, click through them, edit
one, watch the preview update, and trust that the change is on disk — without ever touching the
terminal after launch.

### In scope

**Workspace**
- Open a root folder as a workspace
- File tree sidebar listing `.md` files (other files visible, not editable)
- Sidebar can be collapsed and restored
- Click a file in the tree to open it
- Quick-open dialog (Cmd+P): fuzzy filename match, Enter to open
- Multiple open documents as tabs; close a tab

**Editing and viewing**
- Split view: Markdown source pane + rendered preview pane
- Preview updates live as you type
- Reading mode: hide the editor, render the document full-width
- Toggle between split and reading mode
- Undo / redo
- Find & replace within the current document (Cmd+F)

**Rendering**
- CommonMark + GFM: headings, lists, task lists, emphasis, blockquotes, strikethrough
- Tables rendered correctly
- Fenced code blocks with syntax highlighting
- Images rendered, relative paths resolved against the document's directory
- Links to other `.md` files in the workspace open in a tab
- External links open in the system browser
- Readable default typography, light and dark

**Persistence**
- Debounced autosave to disk
- Atomic writes (temp file + rename) — a crash must never truncate a document
- External change detection: a file modified on disk while open is not silently overwritten

**Launch**
- `medd` CLI: `medd <file.md>`, `medd <folder>`, and bare `medd` to restore the last workspace
- Single instance: a second launch hands its arguments to the running app and exits

### Explicitly not in v0.1

Finder "Open With" registration; the Neovim command; session restore across restarts;
preferences UI; table editing helpers; scroll sync between panes.

---

## v0.2 — Launch everywhere

The original three-way launch requirement, completed.

- Finder integration: `.md` association, "Open With → medd", double-click to open
- **Drag and drop a `.md` file onto the window to open it as a tab.** Deliberately grouped here
  rather than in v0.1: it is the same family as Finder double-click — a file arriving from the
  Finder — and splitting the two across releases would be incoherent. It is *not* covered by D-8's
  rejection of drag-and-drop, which is about inserting images into a document; this is a launch
  path. Today the gesture does nothing at all, which is a missing feature rather than a fault:
  Tauri's own drop handling is enabled, so the WebView does not navigate to the file. A navigation
  would have been a security finding.
- Neovim plugin: a `:Medd` command and suggested keymap that hands the current buffer's file to
  the running app and focuses the window
- All three paths verified to converge on a single running instance
- Session restore: reopen with the previous workspace, tabs, active file, and sidebar state

**Done when:** the user can reach the same running app from Finder, the terminal, and Neovim,
and relaunching feels like the app never left.

---

## v0.3 — Authoring polish

Making it pleasant to actually write in, rather than just read.

- Scroll position synchronised between editor and preview
- Markdown editing helpers: list continuation, bold/italic shortcuts, link insertion
- Table editing helpers: insert row/column, align, tab between cells
- Preferences: autosave delay or manual mode, theme, font size
- File tree operations: new file, rename, delete, new folder
- Filesystem watching so external file additions and deletions appear in the tree live

---

## Later — candidate phases, not committed

Ordered roughly by the user's expressed interest:

- **Live Neovim buffer preview** — stream unsaved buffer changes so medd previews what you are
  typing in Neovim. The most-wanted deferred item; see [decisions.md](decisions.md) D-6
- **Full-text search** across the workspace with a results panel
- **`[[wikilinks]]` and backlinks** — a document graph rather than just resolved links
- **WYSIWYG mode** as a third view alongside split and reading
- **Rich image authoring** — paste from clipboard into an assets folder, drag and drop
- **Mermaid diagrams and LaTeX math** in the preview
- **Export** to HTML or PDF
- **Document outline / table of contents** panel
- **Linux support**
- **Signed and notarised distribution**

## Never (for this product)

Collaboration and multi-user editing; cloud sync or accounts; a plugin system; mobile or web
clients. These would change what the product is.

---

## Release gate

No implementation code is written until the repository contains the agreed design and
architecture documents. This scope document, [requirements.md](requirements.md),
[decisions.md](decisions.md), and [open-questions.md](open-questions.md) are the requirements
half; the architecture half is the next phase's deliverable.
