import { useEffect } from 'react';
import * as projectFile from '../services/projectFile';

/**
 * Standard project keyboard shortcuts:
 *   ⌘N — new untitled project
 *   ⌘O — open existing
 *   ⌘S — save (falls back to Save As if no path)
 *   ⌘⇧S — Save As
 *
 * Disabled while focus is in a text input so users editing field values
 * (BPM, search, etc.) aren't surprised.
 */
export function useProjectShortcuts(): void {
  useEffect(() => {
    async function handle(e: KeyboardEvent) {
      if (isInputFocused(e)) return;
      const mod = e.metaKey || e.ctrlKey;
      if (!mod) return;

      const key = e.key.toLowerCase();
      if (key === 's') {
        e.preventDefault();
        try {
          if (e.shiftKey) {
            await projectFile.saveAs();
          } else {
            await projectFile.save();
          }
        } catch (err) {
          console.error('save failed', err);
        }
      } else if (key === 'o') {
        e.preventDefault();
        try {
          await projectFile.openPicker();
        } catch (err) {
          console.error('open failed', err);
        }
      } else if (key === 'n') {
        e.preventDefault();
        projectFile.newProject();
      }
    }

    window.addEventListener('keydown', handle);
    return () => window.removeEventListener('keydown', handle);
  }, []);
}

function isInputFocused(e: KeyboardEvent): boolean {
  const t = e.target as HTMLElement;
  return (
    t.tagName === 'INPUT' ||
    t.tagName === 'TEXTAREA' ||
    t.tagName === 'SELECT' ||
    t.isContentEditable
  );
}
