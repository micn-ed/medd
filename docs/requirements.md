# medd — Requirements

**Status:** Draft v1 — derived from stakeholder interview, 2026-09-14
**Stakeholder:** eogt04@gmail.com
**Stage:** Requirements only. No architecture, no code.

---

## 1. Product summary

`medd` is a resident desktop application for **reading, browsing, and editing Markdown files**
with a live side-by-side preview. The intended feel is Confluence-like: documents are pleasant
to read, structured content (tables, code, cross-document links) renders properly, and the app
is always open rather than launched per task.

It is deliberately a **local-first, filesystem-backed** tool. There is no server, no account,
no sync. The workspace is a folder on disk; the documents are plain `.md` files.

## 2. Users and context

Single user: a developer working on macOS who lives in the terminal and Neovim, and who keeps
notes and documentation as Markdown in project folders. The app is expected to stay running for
days at a time, so idle resource usage is a first-class concern, not an afterthought.

## 3. Functional requirements

### 3.1 Launch paths

The app must be reachable three ways, all converging on one running instance:

| ID | Requirement | Priority |
|----|-------------|----------|
| L-1 | Launch from Finder — `.md` files can be opened with medd via "Open With", and medd can be set as the default handler for `.md` | Must |
| L-2 | Launch from the terminal — a `medd` CLI command that opens a file (`medd notes.md`) or a folder (`medd ~/docs`), and with no argument restores the last workspace | Must |
| L-3 | Launch from Neovim — a command/keymap that hands the current buffer's file to medd and focuses the window | Must |
| L-4 | All three paths route into the **already-running instance** as a new tab; they never start a second process | Must |

### 3.2 Workspace and navigation

| ID | Requirement | Priority |
|----|-------------|----------|
| W-1 | The app opens a **root folder as a workspace**, not just a single file | Must |
| W-2 | A **file tree sidebar** shows the workspace; clicking a `.md` file opens it | Must |
| W-3 | The sidebar is **collapsible** — it can be hidden to give the document the full window | Must |
| W-4 | Multiple documents open as **tabs** within the single window | Must |
| W-5 | A **quick-open dialog** (Cmd+P) finds files by fuzzy name match and opens the selected one — this is the "button to find and click a file" requirement | Must |
| W-6 | The tree reflects external filesystem changes (files added/renamed/deleted outside the app) | Should |
| W-7 | Workspace state (open tabs, active file, tree expansion, sidebar visibility) is restored on relaunch | Should |
| W-8 | Non-`.md` files are visible in the tree but not editable in v1 | Could |

### 3.3 Editing and viewing

| ID | Requirement | Priority |
|----|-------------|----------|
| E-1 | **Split view**: raw Markdown source on one side, rendered output on the other | Must |
| E-2 | The preview updates **live as you type**, with no manual refresh | Must |
| E-3 | **Reading mode**: a full-width, editor-hidden view of the rendered document for distraction-free reading | Must |
| E-4 | View mode is switchable per tab (split / reading / source-only) | Should |
| E-5 | Scroll position is synchronised between the two panes in split view | Should |
| E-6 | Standard editing affordances: undo/redo, find & replace within the current file (Cmd+F) | Must |
| E-7 | Markdown editing helpers — list continuation, bold/italic shortcuts, table row insertion | Should |
| E-8 | WYSIWYG editing is **explicitly out of scope** for v1; the source pane always shows real Markdown | — |

### 3.4 Rendering ("Confluence-like") features

Confirmed as needed:

| ID | Requirement | Priority |
|----|-------------|----------|
| R-1 | **GFM tables** render correctly in preview, with editor helpers for creating and aligning rows | Must |
| R-2 | **Cross-document links** — clicking a link to another `.md` in the workspace opens it in a tab rather than a browser | Must |
| R-3 | **Fenced code blocks with syntax highlighting** in the preview | Must |
| R-4 | **Basic image rendering** — `![](path.png)` renders, with relative paths resolved against the document's directory. No paste-to-insert, no drag-drop, no resizing in v1 | Should |
| R-5 | Standard CommonMark + GFM: headings, lists, task lists, blockquotes, strikethrough, footnotes | Must |
| R-6 | External (http/https) links open in the system browser | Must |
| R-7 | Typography and spacing tuned for readability — the document should look like a published page, not a raw HTML dump | Must |

Explicitly deferred: `[[wikilinks]]` syntax and backlink panels, Mermaid diagrams, LaTeX math,
comments/annotations, page hierarchy metadata, export to PDF/HTML.

### 3.5 Persistence

| ID | Requirement | Priority |
|----|-------------|----------|
| P-1 | **Autosave** — the buffer is written to disk on a debounce after typing stops (~1s) | Must |
| P-2 | The user is never asked "save changes?" on close; closing a tab is safe | Must |
| P-3 | **External change detection** — if a file changes on disk (e.g. edited in Neovim or by git) while open, the app detects it and does not silently overwrite | Must |
| P-4 | Autosave must not corrupt files on crash — writes are atomic (write-temp-then-rename) | Must |
| P-5 | Autosave behaviour is configurable (delay, or switch to manual) in settings | Could |

### 3.6 Process model

| ID | Requirement | Priority |
|----|-------------|----------|
| I-1 | **Single instance.** One process, one window. A second launch attempt hands its arguments to the running instance and exits | Must |
| I-2 | Closing the window quits the app (no headless daemon in v1) | Must |
| I-3 | The app is safe to leave running indefinitely — no unbounded memory growth from long sessions or many opened tabs | Must |

## 4. Non-functional requirements

| ID | Requirement | Target |
|----|-------------|--------|
| N-1 | Idle memory footprint — the app sits open all day | Modest and stable; no growth over a multi-day session |
| N-2 | Cold start to usable window | Under ~1 second |
| N-3 | Keystroke-to-preview latency | Imperceptible on typical documents; must not degrade badly on large ones |
| N-4 | Platform | macOS is the only supported target for v1. Nothing in the design should gratuitously prevent Linux later |
| N-5 | Distribution | Local build is sufficient for v1; signing/notarisation is a later concern |
| N-6 | The app works fully offline | Absolute |
| N-7 | Files on disk remain plain, portable Markdown — no proprietary sidecar format required to read them | Absolute |

## 5. Process requirement

The GitHub repository must contain the **complete set of design, architecture, and definition
documents before any implementation code is written**. This requirements document is the first
of those.

## 6. Out of scope for v1

Collaboration and multi-user editing; cloud sync; plugin system; git integration; full-text
search across the workspace; export (PDF/HTML/Confluence); WYSIWYG mode; live two-way sync with
Neovim buffers; mobile or web clients; Windows and Linux builds; themes beyond light/dark.
