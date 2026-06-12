import { useEffect, useMemo, useState } from 'react';
import { useProjectStore } from '../stores/project';
import { useUiStore } from '../stores/ui';
import {
  listLibraryPatches,
  loadLibraryPatch,
  type LibraryPatchView,
  type PatchTarget,
} from '../services/patchLibrary';

/**
 * STEAM patch-library browser — Logic-Library-style two-pane modal for
 * Spectrasonics instruments. Click a patch and it loads into the live
 * plug-in instance (audible after the streamer's fade-in); the modal
 * stays open so patches can be auditioned in quick succession.
 */
export function PatchBrowser() {
  const target = useUiStore((s) => s.patchBrowserTarget);
  const close = useUiStore((s) => s.closePatchBrowser);
  if (target === null) return null;
  return <Browser target={target} onClose={close} />;
}

function Browser({ target, onClose }: { target: PatchTarget; onClose(): void }) {
  const defaultPatchName = useProjectStore((s) => s.defaultPatchName);
  const [patches, setPatches] = useState<LibraryPatchView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [category, setCategory] = useState<string | null>(null);
  const [loadingId, setLoadingId] = useState<number | null>(null);
  const [loadedName, setLoadedName] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void listLibraryPatches(target).then((r) => {
      if (cancelled) return;
      if (r.ok) setPatches(r.patches);
      else setError(r.error);
    });
    return () => {
      cancelled = true;
    };
  }, [target]);

  // Group rail: first two category segments ("Keyboards / Acoustic Pianos").
  const groups = useMemo(() => {
    if (!patches) return [];
    const counts = new Map<string, number>();
    for (const p of patches) {
      const g = p.category.split('/').slice(0, 2).join(' / ') || p.library;
      counts.set(g, (counts.get(g) ?? 0) + 1);
    }
    return [...counts.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  }, [patches]);

  const visible = useMemo(() => {
    if (!patches) return [];
    const q = query.trim().toLowerCase();
    return patches.filter((p) => {
      if (category !== null) {
        const g = p.category.split('/').slice(0, 2).join(' / ') || p.library;
        if (g !== category) return false;
      }
      if (q && !`${p.name} ${p.category}`.toLowerCase().includes(q)) return false;
      return true;
    });
  }, [patches, query, category]);

  const activeName = loadedName ?? defaultPatchName;

  const pick = async (p: LibraryPatchView) => {
    if (loadingId !== null) return;
    setLoadingId(p.id);
    setError(null);
    const r = await loadLibraryPatch(target, p.id);
    setLoadingId(null);
    if (r.ok) setLoadedName(r.patchName ?? p.name);
    else setError(r.error);
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
      onClick={onClose}
      onKeyDown={(e) => {
        if (e.key === 'Escape') onClose();
      }}
    >
      <div
        className="bg-daw-panel border border-daw-border rounded-lg w-[720px] h-[70vh] flex flex-col shadow-xl overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center gap-3 px-4 py-3 border-b border-daw-border shrink-0">
          <h3 className="text-sm font-medium text-[#e0e0e0]">音色库</h3>
          {patches && (
            <span className="lcd-label !text-[9px] text-[#7c776c]">{patches.length} 个音色</span>
          )}
          <input
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="搜索音色…"
            autoFocus
            className="flex-1 max-w-[260px] ml-auto bg-daw-control border border-daw-border rounded px-3 py-1.5 text-xs text-[#e0e0e0] placeholder-[#555] outline-none focus:border-daw-accent"
          />
          <button
            type="button"
            onClick={onClose}
            className="text-[#666] hover:text-[#ccc] text-lg leading-none px-1"
            aria-label="关闭"
          >
            ×
          </button>
        </div>

        {error !== null && (
          <div className="px-4 py-2 text-[11px] text-red-400 border-b border-daw-border shrink-0">
            {error}
          </div>
        )}

        {patches === null && error === null && (
          <div className="flex-1 flex items-center justify-center text-xs text-[#6b675f]">
            正在扫描音色库…
          </div>
        )}

        {patches !== null && (
          <div className="flex-1 flex min-h-0">
            {/* Category rail */}
            <div className="w-52 shrink-0 border-r border-daw-border overflow-y-auto py-1.5">
              <RailItem
                label="全部"
                count={patches.length}
                active={category === null}
                onClick={() => setCategory(null)}
              />
              {groups.map(([g, n]) => (
                <RailItem
                  key={g}
                  label={g}
                  count={n}
                  active={category === g}
                  onClick={() => setCategory(g)}
                />
              ))}
            </div>

            {/* Patch list */}
            <div className="flex-1 overflow-y-auto py-1.5">
              {visible.map((p, i) => {
                const prev = visible[i - 1];
                const showHeader = !prev || prev.category !== p.category;
                const isActive = activeName !== null && p.name === activeName;
                const isLoading = loadingId === p.id;
                return (
                  <div key={p.id}>
                    {showHeader && (
                      <div className="lcd-label !text-[9px] text-[#6b675f] px-3 pt-2.5 pb-1 truncate">
                        {p.category || p.library}
                      </div>
                    )}
                    <button
                      type="button"
                      onClick={() => void pick(p)}
                      disabled={loadingId !== null}
                      className={`w-full text-left px-3 py-1.5 text-xs flex items-center gap-2 transition-colors ${
                        isActive
                          ? 'bg-daw-accent/20 text-daw-accent-hi'
                          : 'text-[#cfcbc4] hover:bg-daw-control/60'
                      } disabled:opacity-60`}
                    >
                      <span className="flex-1 truncate">{p.name}</span>
                      {isLoading && <span className="text-[10px] text-[#8a857b]">加载中…</span>}
                      {isActive && !isLoading && (
                        <span className="w-1.5 h-1.5 rounded-full bg-daw-accent shrink-0" />
                      )}
                    </button>
                  </div>
                );
              })}
              {visible.length === 0 && (
                <div className="text-xs text-[#6b675f] text-center py-10">没有匹配的音色</div>
              )}
            </div>
          </div>
        )}

        <div className="px-4 py-2 border-t border-daw-border text-[10px] text-[#6b675f] shrink-0">
          点击即加载到当前乐器 —— 采样流入需要一两秒，可以边播放边换音色试听。
        </div>
      </div>
    </div>
  );
}

function RailItem({
  label,
  count,
  active,
  onClick,
}: {
  label: string;
  count: number;
  active: boolean;
  onClick(): void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`w-full text-left px-3 py-1.5 text-xs flex items-center gap-2 transition-colors ${
        active ? 'bg-daw-accent/15 text-daw-accent-hi' : 'text-[#a59f93] hover:bg-daw-control/60'
      }`}
    >
      <span className="flex-1 truncate">{label}</span>
      <span className="text-[9px] font-lcd text-[#6b675f] tabular-nums">{count}</span>
    </button>
  );
}
