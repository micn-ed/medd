<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { Tree } from './tree'

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

  let openPath = $state<string | null>(null)
  let openContent = $state('')
  let error = $state('')

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
      workspaceRoot = info.root
      workspaceName = info.name
      openPath = null
      openContent = ''
    } catch (e) {
      error = formatError(e)
    }
  }

  async function openFile(path: string) {
    error = ''
    try {
      const result = await invoke<ReadResult>('document_read', { path })
      openPath = path
      openContent = result.content
    } catch (e) {
      error = formatError(e)
    }
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
      {:else if openPath}
        <p class="path">{openPath}</p>
        <pre>{openContent}</pre>
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
    overflow-y: auto;
    padding: 1rem;
  }

  .content .path {
    font-family: ui-monospace, monospace;
    font-size: 0.85em;
    opacity: 0.7;
    margin: 0 0 0.75rem;
  }

  .content pre {
    white-space: pre-wrap;
    word-wrap: break-word;
    font-family: ui-monospace, monospace;
  }

  .hint {
    opacity: 0.6;
  }

  .error {
    color: var(--error, #c0392b);
  }
</style>
