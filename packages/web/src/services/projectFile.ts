/**
 * Project-file UX glue: thin wrappers over the Tauri session commands
 * that handle dialog flow + recent-files updates. Frontend components
 * call these instead of `invoke` directly so the dialog logic stays
 * in one place.
 *
 * Each function uses the `@tauri-apps/plugin-dialog` save/open pickers,
 * then dispatches to a `cmd_project_*` Tauri command. All errors surface
 * as thrown exceptions; callers handle UI feedback.
 */

import { invoke } from '@tauri-apps/api/core';
import { open as openDialog, save as saveDialog } from '@tauri-apps/plugin-dialog';
import { useProjectFileStore } from '../stores/projectFile';

const SESSION_FILTERS = [{ name: 'moonlitt project', extensions: ['mlsession'] }];

interface ProjectStateLike {
  bpm: number;
  playing: boolean;
  defaultInstrumentPath: string | null;
  midi: unknown | null;
  overrides: unknown[];
}

export interface RecentList {
  recent: string[];
  lastOpened: string | null;
}

/** Open a save dialog and write the current engine state to disk. */
export async function saveAs(): Promise<string | null> {
  const path = await saveDialog({
    title: '保存项目',
    defaultPath: 'untitled.mlsession',
    filters: SESSION_FILTERS,
  });
  if (!path) return null;
  await invoke<void>('cmd_project_save_as', { path });
  useProjectFileStore.getState().setPath(path);
  return path;
}

/**
 * Save to the currently-open path. If none is set (untitled scratch),
 * falls back to Save As. Returns the path saved to.
 */
export async function save(): Promise<string | null> {
  const current = useProjectFileStore.getState().path;
  if (!current) return saveAs();
  await invoke<void>('cmd_project_save_as', { path: current });
  useProjectFileStore.getState().markClean();
  return current;
}

/** Show an open dialog and load the chosen project. */
export async function openPicker(): Promise<string | null> {
  const path = await openDialog({
    title: '打开项目',
    multiple: false,
    directory: false,
    filters: SESSION_FILTERS,
  });
  if (!path || typeof path !== 'string') return null;
  await openPath(path);
  return path;
}

/** Load a specific project file (used by Recent menu). */
export async function openPath(path: string): Promise<void> {
  await invoke<ProjectStateLike>('cmd_project_open', { path });
  useProjectFileStore.getState().setPath(path);
}

/** Read the recent-projects list from app data. */
export async function recentList(): Promise<RecentList> {
  return invoke<RecentList>('cmd_project_recent_list');
}

export async function clearRecent(): Promise<void> {
  await invoke<void>('cmd_project_clear_recent');
}

/**
 * Begin a fresh untitled project. Backend has no "new" concept (the
 * engine starts blank); this just clears the open-path so the next ⌘S
 * triggers Save As.
 */
export function newProject(): void {
  useProjectFileStore.getState().setPath(null);
  // Note: this does NOT wipe engine state. The user may explicitly want
  // to start from their current default instrument as a template. To
  // truly reset, the backend would need a `cmd_project_reset` — added
  // later if the workflow demands it.
}
