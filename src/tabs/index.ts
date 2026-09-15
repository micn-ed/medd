export { default as TabBar } from './TabBar.svelte'
export {
  allTabs,
  activeTab,
  activeTabPath,
  getTab,
  editorStateFor,
  registerMountedView,
  unregisterMountedView,
  openTab,
  closeTab,
  closeAllTabs,
  setActiveTab,
  setActiveTabViewMode,
  setOnDocChanged,
  setOnTabClosing,
  currentGeneration,
  markSynced,
  markConflict,
  markDetached,
  applyExternalContent,
  resolveConflictKeepMine,
} from './tabs.svelte'
export type { Tab, ViewMode, Conflict } from './tabs.svelte'
