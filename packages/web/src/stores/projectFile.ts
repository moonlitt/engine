import { create } from 'zustand';

/**
 * Tracks the on-disk project file the user is currently working with.
 * Mirrors the "current document" idea every native DAW has:
 *   - `path`: absolute path to the open `.mlsession` (null = untitled scratch)
 *   - `dirty`: true if the engine state has diverged from disk since the
 *     last successful Open or Save. UI uses this to disambiguate ⌘S
 *     (overwrite current) vs first-save-needs-Save-As, and to show the
 *     "•" modified marker in the title.
 */
interface ProjectFileStore {
  path: string | null;
  dirty: boolean;
  setPath(p: string | null): void;
  markDirty(): void;
  markClean(): void;
}

export const useProjectFileStore = create<ProjectFileStore>((set) => ({
  path: null,
  dirty: false,
  setPath(p) { set({ path: p, dirty: false }); },
  markDirty() { set({ dirty: true }); },
  markClean() { set({ dirty: false }); },
}));
