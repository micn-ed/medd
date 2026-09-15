<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import type { EditorView } from '@codemirror/view'
  import { Tree } from './tree'
  import { Editor } from './editor'
  import { Preview, dirname } from './render'
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
  import { ConflictBanner, initDocSync, reload, keepMine } from './doc'

  initDocSync()

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
  let sidebarCollapsed = $state(false)
  let error = $state('')

  let tab = $derived(activeTab())
  let documentDir = $derived(tab ? dirname(tab.path) : '')

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
    error = ''
    try {
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

<div class="app">
  <header>
    <button onclick={pickFolder}>Open Folder…</button>
    {#if workspaceRoot}
      <button onclick={() => (sidebarCollapsed = !sidebarCollapsed)}>
        {sidebarCollapsed ? 'Show sidebar' : 'Hide sidebar'}
      </button>
      <span class="workspace-name">{workspaceName}</span>
    {/if}
  </header>

  <div class="body">
    {#if workspaceRoot && !sidebarCollapsed}
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
              onclick={() => {
                setActiveTabViewMode('reading')
                sidebarCollapsed = true
              }}
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
