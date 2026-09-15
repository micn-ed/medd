<script lang="ts">
  // D-11: shown only while a tab is both dirty and has received a genuine external change (or
  // had an autosave rejected as a conflict) — the rare, genuinely ambiguous case where a prompt
  // earns its interruption, as opposed to the clean case, which reloads silently with no banner
  // at all.
  //
  // The consequence line exists because "Keep mine" is the only control in medd whose effect is
  // to destroy someone else's version of the document, and with `Diff…` deferred past v0.1 this
  // sentence is the only thing telling the user that before they click (increment-7 review).
  let { onReload, onKeepMine }: { onReload: () => void; onKeepMine: () => void } = $props()
</script>

<div class="conflict-banner" role="alert">
  <div class="text">
    <p class="headline">This file changed on disk.</p>
    <p class="consequence">
      Reloading discards your unsaved edits. Keeping yours overwrites the version on disk.
    </p>
  </div>
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

  .text {
    display: flex;
    flex-direction: column;
    gap: 0.15em;
  }

  .headline {
    margin: 0;
    font-weight: 600;
  }

  .consequence {
    margin: 0;
    opacity: 0.85;
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
