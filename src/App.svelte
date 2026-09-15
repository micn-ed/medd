<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { listen } from '@tauri-apps/api/event'
  import type { EditorView } from '@codemirror/view'
  import { Tree } from './tree'
  import { Editor } from './editor'
  import { Preview, dirname } from './render'
  import { sidebarLayout } from './sidebar'
  import { QuickOpenDialog } from './quickopen'
  import {
    TabBar,
    allTabs,
    activeTab,
    activeTabPath,
    editorStateFor,
    openTab,
    closeTab,
    closeAllTabs,
    setActiveTab,
    setActiveTabViewMode,
    registerMountedView,
    unregisterMountedView,
  } from './tabs'
  import {
    ConflictBanner,
    initDocSync,
    reload,
    keepMine,
    waitForQuiescence,
    isQuitInProgress,
  } from './doc'

  initDocSync()

  // Cmd+W (medd's own menu, main.rs's `build_menu` — Menu::default's binding quits the app under
  // I-2, which every other tabbed editor on the platform reserves for close-tab). `closeTab`
  // already flushes any pending autosave for the tab it removes, and does nothing if no tab is
  // active, so there's nothing else this needs to do.
  void listen('menu:close-tab', () => {
    const path = activeTabPath()
    if (path) closeTab(path)
  })

  interface WorkspaceInfo {
    root: string
    name: string
  }

  interface ReadResult {
    content: string
    hash: string
  }

  let workspaceRoot = $state<string | null>(null)
  let workspaceName = $state('')
  // The user's own answer to "is the file tree useful to me right now?" (W-3) — workspace-level,
  // durable, and written ONLY by the sidebar toggle below. Never assign to this from anywhere
  // else: reading mode used to (`sidebarCollapsed = true`) and nothing ever unset it, which is
  // what made a stale collapse look like "the user's choice" long after they'd switched tabs.
  // Named for what it is (a stored preference) rather than for what's on screen, on purpose —
  // architecture.md §5's pattern: when two inputs determine a presentational fact, store the
  // inputs and derive the fact, because a name that reads like live state invites exactly this bug.
  let sidebarHiddenByUser = $state(false)
  let error = $state('')

  let tab = $derived(activeTab())
  let documentDir = $derived(tab ? dirname(tab.path) : '')
  // `sidebar.ts`'s own header explains why `visible` is defined in terms of `showToggle` rather
  // than as a second, separately-repeated condition: that's what keeps a control that's present
  // but does nothing (reading mode) from becoming possible again the next time a condition is
  // added to one but not the other.
  let { visible: sidebarVisible, showToggle: showSidebarToggle } = $derived(
    sidebarLayout(workspaceRoot, sidebarHiddenByUser, tab?.viewMode),
  )

  function formatError(e: unknown): string {
    if (e && typeof e === 'object' && 'kind' in e) {
      const err = e as { kind: string; message?: string; path?: string }
      if (err.kind === 'Io') return `${err.path}: ${err.message}`
      if (err.kind === 'NotUtf8') return `${err.path}: not valid UTF-8`
      if (err.kind === 'Conflict') return 'File changed on disk'
      return err.kind
    }
    return String(e)
  }

  async function pickFolder() {
    error = ''
    const picked = await invoke<string | null>('workspace_pick')
    if (!picked) return
    try {
      const info = await invoke<WorkspaceInfo>('workspace_open', { path: picked })
      // A new workspace's tree has no relationship to whatever was open before.
      closeAllTabs()
      workspaceRoot = info.root
      workspaceName = info.name
    } catch (e) {
      error = formatError(e)
    }
  }

  async function openFile(path: string) {
    // Shutdown latch (plan-v0.1.md's fifth blocker): opening a tab is new work, and once quit's
    // flush has begun medd accepts none, not just new writes. Harmless today by coincidence — a
    // freshly read tab starts clean, so nothing would schedule a write regardless, and
    // `scheduleAutosave` is separately latched — but relying on that coincidence is exactly the
    // shape that stops being true the next time either of those changes for an unrelated reason.
    if (isQuitInProgress()) return
    error = ''
    try {
      // If this path was just closed with a write still airborne (autosave issues a write
      // regardless of the tab's own lifetime — doc/doc.ts), reading now could land on disk a
      // moment before that write commits, handing this fresh tab a baseline the write's own
      // settling immediately makes stale: a conflict banner on a file the user just opened and
      // has not touched, offering their own earlier text back as though it were someone else's
      // change (increment-7 review). Waiting for quiescence first means the read always reflects
      // what's actually, finally on disk for this path.
      await waitForQuiescence(path)
      const result = await invoke<ReadResult>('document_read', { path })
      openTab(path, result.content, result.hash, workspaceRoot)
    } catch (e) {
      error = formatError(e)
    }
  }

  function onMountedView(path: string, view: EditorView | null) {
    if (view) registerMountedView(path, view)
    else unregisterMountedView(path)
  }
</script>

<QuickOpenDialog onOpenFile={openFile} />

<div class="app">
  <header>
    <button onclick={pickFolder}>Open Folder…</button>
    {#if workspaceRoot}
      {#if showSidebarToggle}
        <button onclick={() => (sidebarHiddenByUser = !sidebarHiddenByUser)}>
          {sidebarHiddenByUser ? 'Show sidebar' : 'Hide sidebar'}
        </button>
      {/if}
      <span class="workspace-name">{workspaceName}</span>
    {/if}
  </header>

  <div class="body">
    {#if workspaceRoot && sidebarVisible}
      <nav class="sidebar">
        <Tree path={workspaceRoot} onOpenFile={openFile} />
      </nav>
    {/if}

    <main class="content">
      {#if error}
        <p class="error">{error}</p>
      {:else if tab}
        <TabBar
          tabs={allTabs()}
          activePath={activeTabPath()}
          onSelect={setActiveTab}
          onClose={closeTab}
        />

        {#if tab.conflict}
          <ConflictBanner onReload={() => reload(tab.path)} onKeepMine={() => keepMine(tab.path)} />
        {/if}

        <div class="toolbar">
          <p class="path">{tab.path}</p>
          <div class="mode-toggle" role="group" aria-label="View mode">
            <button
              class:active={tab.viewMode === 'source'}
              aria-pressed={tab.viewMode === 'source'}
              onclick={() => setActiveTabViewMode('source')}
            >
              Source
            </button>
            <button
              class:active={tab.viewMode === 'split'}
              aria-pressed={tab.viewMode === 'split'}
              onclick={() => setActiveTabViewMode('split')}
            >
              Split
            </button>
            <button
              class:active={tab.viewMode === 'reading'}
              aria-pressed={tab.viewMode === 'reading'}
              onclick={() => setActiveTabViewMode('reading')}
            >
              Reading
            </button>
          </div>
        </div>

        {#if tab.viewMode === 'split'}
          <div class="split">
            <div class="editor-pane">
              {#key tab.path}
                <Editor
                  state={editorStateFor(tab.path)}
                  onMountedView={(view) => onMountedView(tab.path, view)}
                />
              {/key}
            </div>
            <div class="preview-pane">
              <Preview text={tab.currentText} {documentDir} {workspaceRoot} onOpenFile={openFile} />
            </div>
          </div>
        {:else if tab.viewMode === 'source'}
          <div class="single-pane">
            {#key tab.path}
              <Editor
                state={editorStateFor(tab.path)}
                onMountedView={(view) => onMountedView(tab.path, view)}
              />
            {/key}
          </div>
        {:else}
          <div class="single-pane">
            <Preview
              text={tab.currentText}
              {documentDir}
              {workspaceRoot}
              onOpenFile={openFile}
              reading
            />
          </div>
        {/if}
      {:else if workspaceRoot}
        <p class="hint">Click a Markdown file in the sidebar to view it.</p>
      {:else}
        <p class="hint">Open a folder to get started.</p>
      {/if}
    </main>
  </div>
</div>

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    font-family: -apple-system, BlinkMacSystemFont, sans-serif;
  }

  header {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid var(--border, #d8d8d8);
  }

  .workspace-name {
    font-weight: 600;
    opacity: 0.8;
  }

  .body {
    flex: 1;
    display: flex;
    min-height: 0;
  }

  .sidebar {
    width: 240px;
    flex-shrink: 0;
    overflow-y: auto;
    padding: 0.5rem;
    border-right: 1px solid var(--border, #d8d8d8);
  }

  .content {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    flex-shrink: 0;
    padding: 0.75rem 1rem 0;
  }

  .content .path {
    font-family: ui-monospace, monospace;
    font-size: 0.85em;
    opacity: 0.7;
    margin: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .mode-toggle {
    display: flex;
    flex-shrink: 0;
    border: 1px solid var(--border, #d8d8d8);
    border-radius: 6px;
    overflow: hidden;
  }

  .mode-toggle button {
    font: inherit;
    font-size: 0.85em;
    padding: 0.3em 0.8em;
    border: none;
    background: none;
    color: inherit;
    cursor: pointer;
  }

  .mode-toggle button:not(:last-child) {
    border-right: 1px solid var(--border, #d8d8d8);
  }

  .mode-toggle button.active {
    background: var(--border, #d8d8d8);
    font-weight: 600;
  }

  .split,
  .single-pane {
    flex: 1;
    min-height: 0;
    padding: 0.75rem 1rem 1rem;
  }

  .split {
    display: flex;
    gap: 1rem;
  }

  .editor-pane,
  .preview-pane {
    flex: 1;
    min-width: 0;
    min-height: 0;
  }

  .preview-pane {
    border-left: 1px solid var(--border, #d8d8d8);
    padding-left: 1rem;
  }

  .hint {
    opacity: 0.6;
    padding: 1rem;
  }

  .error {
    color: var(--error, #c0392b);
    padding: 1rem;
  }
</style>
