import { useState, useCallback, useEffect, useMemo } from 'react';
import type { PluginInfo } from '@moonlitt/protocol';
import { usePluginsStore } from '../stores/plugins';
import { useSessionStore } from '../stores/session';

interface InstrumentSelectorProps {
  open: boolean;
  onLoad: (path: string) => void;
  onClose: () => void;
}

// Map raw Rust enum debug strings to display labels and badge colors.
const FORMAT_META: Record<string, { label: string; badge: string }> = {
  Sf2: { label: 'SF2 Soundfont', badge: 'bg-emerald-500/20 text-emerald-300' },
  Vst3: { label: 'VST3', badge: 'bg-sky-500/20 text-sky-300' },
  Clap: { label: 'CLAP', badge: 'bg-violet-500/20 text-violet-300' },
};

const FORMAT_ORDER: readonly string[] = ['Sf2', 'Vst3', 'Clap'];

export function InstrumentSelector({ open, onLoad, onClose }: InstrumentSelectorProps) {
  const send = useSessionStore((s) => s.send);
  const plugins = usePluginsStore((s) => s.list);
  const scanning = usePluginsStore((s) => s.scanning);
  const setScanning = usePluginsStore((s) => s.setScanning);

  const [query, setQuery] = useState('');
  const [path, setPath] = useState('');

  // Trigger a scan the first time the dialog opens (and the cache is empty).
  useEffect(() => {
    if (!open) return;
    if (plugins.length === 0 && !scanning) {
      setScanning(true);
      send({ type: 'plugins.scan' });
    }
  }, [open, plugins.length, scanning, setScanning, send]);

  const handleLoadFromList = useCallback(
    (plugin: PluginInfo) => {
      onLoad(plugin.path);
      setQuery('');
      setPath('');
    },
    [onLoad],
  );

  const handleLoadFromPath = useCallback(() => {
    const trimmed = path.trim();
    if (trimmed.length === 0) return;
    onLoad(trimmed);
    setQuery('');
    setPath('');
  }, [path, onLoad]);

  const handleRescan = useCallback(() => {
    setScanning(true);
    send({ type: 'plugins.scan', force: true });
  }, [send, setScanning]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    },
    [onClose],
  );

  // Filter + group
  const grouped = useMemo(() => {
    const q = query.trim().toLowerCase();
    const filtered = q
      ? plugins.filter(
          (p) =>
            p.name.toLowerCase().includes(q) ||
            p.path.toLowerCase().includes(q) ||
            p.format.toLowerCase().includes(q),
        )
      : plugins;
    const map = new Map<string, PluginInfo[]>();
    for (const p of filtered) {
      const arr = map.get(p.format) ?? [];
      arr.push(p);
      map.set(p.format, arr);
    }
    return FORMAT_ORDER
      .filter((fmt) => map.has(fmt))
      .map((fmt) => ({ format: fmt, items: map.get(fmt)!.sort((a, b) => a.name.localeCompare(b.name)) }))
      .concat(
        Array.from(map.keys())
          .filter((fmt) => !FORMAT_ORDER.includes(fmt))
          .map((fmt) => ({ format: fmt, items: map.get(fmt)!.sort((a, b) => a.name.localeCompare(b.name)) })),
      );
  }, [plugins, query]);

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
      onClick={onClose}
      onKeyDown={handleKeyDown}
    >
      <div
        className="bg-daw-panel border border-daw-border rounded-lg w-[560px] max-h-[80vh] flex flex-col shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-daw-border">
          <h3 className="text-sm font-medium text-[#e0e0e0]">Load Instrument</h3>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={handleRescan}
              disabled={scanning}
              className="text-[10px] px-2 py-1 rounded bg-daw-control hover:bg-daw-border text-[#aaa] transition-colors disabled:opacity-50"
              title="Re-scan plugin folders"
            >
              {scanning ? 'Scanning…' : 'Rescan'}
            </button>
            <button
              type="button"
              onClick={onClose}
              className="text-[#666] hover:text-[#ccc] text-lg leading-none px-1"
              aria-label="Close"
            >
              ×
            </button>
          </div>
        </div>

        {/* Search */}
        <div className="px-4 pt-3">
          <input
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search plugins…"
            autoFocus
            className="w-full bg-daw-control border border-daw-border rounded px-3 py-2 text-xs text-[#e0e0e0] placeholder-[#555] outline-none focus:border-daw-accent"
          />
        </div>

        {/* List */}
        <div className="flex-1 overflow-y-auto px-2 py-2 min-h-[180px]">
          {scanning && plugins.length === 0 && (
            <div className="text-xs text-[#666] text-center py-8">Scanning system plugin folders…</div>
          )}
          {!scanning && plugins.length === 0 && (
            <div className="text-xs text-[#666] text-center py-8">
              No plugins found. Use the path field below to load a file directly.
            </div>
          )}
          {grouped.length === 0 && plugins.length > 0 && (
            <div className="text-xs text-[#666] text-center py-8">
              No plugins match “{query}”.
            </div>
          )}
          {grouped.map((group) => {
            const meta = FORMAT_META[group.format] ?? { label: group.format, badge: 'bg-[#444] text-[#aaa]' };
            return (
              <div key={group.format} className="mb-3">
                <div className="text-[9px] uppercase tracking-wider text-[#777] px-2 mb-1">
                  {meta.label} ({group.items.length})
                </div>
                <div className="flex flex-col">
                  {group.items.map((p) => (
                    <button
                      key={p.path}
                      type="button"
                      onClick={() => handleLoadFromList(p)}
                      className="flex items-center gap-2 px-2 py-1.5 rounded text-left hover:bg-daw-control transition-colors group"
                      title={p.path}
                    >
                      <span className={`text-[9px] font-mono px-1.5 py-0.5 rounded shrink-0 ${meta.badge}`}>
                        {group.format.toUpperCase()}
                      </span>
                      <span className="text-xs text-[#ddd] truncate flex-1">{p.name}</span>
                      <span className="text-[9px] text-[#555] truncate max-w-[180px] hidden group-hover:inline">
                        {p.path}
                      </span>
                    </button>
                  ))}
                </div>
              </div>
            );
          })}
        </div>

        {/* Path fallback */}
        <div className="border-t border-daw-border px-4 py-3">
          <div className="text-[9px] uppercase tracking-wider text-[#777] mb-1.5">
            Or paste a path
          </div>
          <div className="flex gap-2">
            <input
              type="text"
              value={path}
              onChange={(e) => setPath(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') handleLoadFromPath();
              }}
              placeholder="/path/to/instrument.sf2  or  /Library/Audio/Plug-Ins/VST3/Foo.vst3"
              className="flex-1 bg-daw-control border border-daw-border rounded px-3 py-1.5 text-xs text-[#e0e0e0] placeholder-[#555] outline-none focus:border-daw-accent"
            />
            <button
              type="button"
              onClick={handleLoadFromPath}
              disabled={path.trim().length === 0}
              className="px-3 py-1.5 text-xs rounded bg-daw-accent hover:bg-daw-accent/80 text-white transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
            >
              Load
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
