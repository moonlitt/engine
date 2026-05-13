import { useCallback, useEffect, useRef, useState } from 'react';
import * as projectFile from '../services/projectFile';
import type { RecentList } from '../services/projectFile';

/**
 * Project dropdown in the top bar. Renders as a single button labelled
 * with the open project name (or "无项目"); clicking expands to:
 *   - New / Open / Save / Save As
 *   - Recent files (up to 10), most-recent first
 *   - Clear recent files
 *
 * Tauri 2 does have a native menu API but it's per-window and requires
 * a separate menu-builder pass. For a single-window app this in-content
 * menu keeps things in one place and looks like Logic's project chip.
 */
export function ProjectMenu({ currentPath, dirty }: { currentPath: string | null; dirty: boolean }) {
  const [open, setOpen] = useState(false);
  const [recent, setRecent] = useState<RecentList>({ recent: [], lastOpened: null });
  const menuRef = useRef<HTMLDivElement>(null);

  const refreshRecent = useCallback(async () => {
    try {
      setRecent(await projectFile.recentList());
    } catch (e) {
      console.error('recent list:', e);
    }
  }, []);

  useEffect(() => {
    if (open) {
      void refreshRecent();
    }
  }, [open, refreshRecent]);

  // Close on outside click.
  useEffect(() => {
    if (!open) return;
    function handle(e: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    window.addEventListener('mousedown', handle);
    return () => window.removeEventListener('mousedown', handle);
  }, [open]);

  const label = currentPath ? fileName(currentPath) : '无项目';
  const wrappedAction = (fn: () => Promise<unknown> | unknown) => async () => {
    setOpen(false);
    try {
      await fn();
    } catch (e) {
      console.error('project action failed:', e);
    }
  };

  return (
    <div className="relative" ref={menuRef}>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="px-3 py-1.5 rounded bg-daw-control hover:bg-daw-border text-[#e0e0e0] text-xs flex items-center gap-1.5"
        title="项目菜单"
      >
        <span>📁</span>
        <span className="font-medium truncate max-w-[160px]">
          {label}
          {dirty && <span className="text-daw-accent ml-1">●</span>}
        </span>
        <span className="text-[#888] text-[10px] ml-1">▾</span>
      </button>

      {open && (
        <div className="absolute right-0 mt-1 w-64 bg-daw-panel border border-daw-border rounded shadow-lg py-1 z-50">
          <MenuItem label="新建" shortcut="⌘N" onClick={wrappedAction(projectFile.newProject)} />
          <MenuItem label="打开…" shortcut="⌘O" onClick={wrappedAction(projectFile.openPicker)} />
          <MenuItem label="保存" shortcut="⌘S" onClick={wrappedAction(projectFile.save)} />
          <MenuItem label="另存为…" shortcut="⌘⇧S" onClick={wrappedAction(projectFile.saveAs)} />

          {recent.recent.length > 0 && (
            <>
              <Separator />
              <SubHeader label="最近打开" />
              {recent.recent.map((p) => (
                <MenuItem
                  key={p}
                  label={fileName(p)}
                  hint={parentDir(p)}
                  onClick={wrappedAction(() => projectFile.openPath(p))}
                />
              ))}
              <Separator />
              <MenuItem
                label="清空最近列表"
                onClick={wrappedAction(async () => {
                  await projectFile.clearRecent();
                  await refreshRecent();
                })}
              />
            </>
          )}
        </div>
      )}
    </div>
  );
}

function MenuItem({
  label,
  shortcut,
  hint,
  onClick,
}: {
  label: string;
  shortcut?: string;
  hint?: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="w-full text-left px-3 py-1.5 text-xs hover:bg-daw-control text-[#e0e0e0] flex items-center justify-between gap-3"
    >
      <span className="flex flex-col min-w-0">
        <span className="truncate">{label}</span>
        {hint && <span className="text-[10px] text-[#666] truncate">{hint}</span>}
      </span>
      {shortcut && <span className="text-[10px] text-[#666] font-mono shrink-0">{shortcut}</span>}
    </button>
  );
}

function Separator() {
  return <div className="h-px bg-daw-border my-1" />;
}

function SubHeader({ label }: { label: string }) {
  return <div className="px-3 py-1 text-[10px] uppercase tracking-wider text-[#666]">{label}</div>;
}

function fileName(p: string): string {
  const last = p.split('/').pop() ?? p;
  return last.replace(/\.mlsession$/i, '');
}

function parentDir(p: string): string {
  const parts = p.split('/');
  parts.pop();
  return parts.slice(-2).join('/');
}
