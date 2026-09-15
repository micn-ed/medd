<script lang="ts">
  // D-11: shown only while a tab is both dirty and has received a genuine external change (or
  // had an autosave rejected as a conflict) — the rare, genuinely ambiguous case where a prompt
  // earns its interruption, as opposed to the clean case, which reloads silently with no banner
  // at all.
  let { onReload, onKeepMine }: { onReload: () => void; onKeepMine: () => void } = $props()
</script>

<div class="conflict-banner" role="alert">
  <span>This file changed on disk.</span>
  <div class="actions">
    <button onclick={onReload}>Reload</button>
    <button onclick={onKeepMine}>Keep mine</button>
  </div>
</div>

<style>
  .conflict-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.5rem 1rem;
    background: var(--conflict-bg);
    color: var(--conflict-fg);
    font-size: 0.85em;
  }

  .actions {
    display: flex;
    gap: 0.5rem;
    flex-shrink: 0;
  }

  .conflict-banner button {
    font: inherit;
    padding: 0.2em 0.8em;
    border: 1px solid currentColor;
    border-radius: 4px;
    background: none;
    color: inherit;
    cursor: pointer;
  }
</style>
