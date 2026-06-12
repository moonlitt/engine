import { useState, useCallback, useEffect, useMemo } from 'react';
import type { PluginInfo } from '@moonlitt/protocol';
import { usePluginsStore } from '../stores/plugins';
import { useSessionStore } from '../stores/session';

interface InstrumentSelectorProps {
  open: boolean;
  /** Which slot is being filled — the DEFAULT slot is the project's
   *  multi-channel GM bed, so single-instrument plug-ins get a warning
   *  there (all channels would play the same sound). */
  targetKind: 'default' | 'override' | null;
  onLoad: (path: string) => void;
  onClose: () => void;
}

// Map raw Rust enum debug strings to display labels and badge colors.
const FORMAT_META: Record<string, { label: string; badge: string }> = {
  Sf2: { label: 'SF2 SoundFont (自带 128 套 GM 音色)', badge: 'bg-emerald-500/20 text-emerald-300' },
  Vst3: { label: 'VST3 插件 (支持原生界面; Keyscape/Omnisphere 还可直接浏览音色库)', badge: 'bg-sky-500/20 text-sky-300' },
  Clap: { label: 'CLAP 插件', badge: 'bg-amber-500/20 text-amber-300' },
};

/** Default-slot guidance overrides: the bed must cover 16 channels. */
const FORMAT_META_DEFAULT_SLOT: Record<string, string> = {
  Sf2: 'SF2 SoundFont — GM 底座：按 MIDI Program Change 自动给每个通道配音色',
  Vst3: 'VST3 单乐器插件 — 仅在没有任何 SF2 时可选：所有通道会发同一种声音',
  Clap: 'CLAP 插件 — 同上，单乐器',
};

const FORMAT_ORDER: readonly string[] = ['Sf2', 'Vst3', 'Clap'];

export function InstrumentSelector({ open, targetKind, onLoad, onClose }: InstrumentSelectorProps) {
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

  // Filter + group. Effect-only plug-ins (FX-Omnisphere,
  // SurgeEffectsBank) can't act as a sound source — hide them here;
  // they stay available as channel/bus inserts.
  //
  // The DEFAULT slot is the project's GM bed and must cover all 16
  // channels, so it only lists multi-timbral sources (SF2) — a
  // single-instrument plug-in there would turn every channel into one
  // sound. Keyscape-class plug-ins belong on channel overrides. Escape
  // hatch: machines with no SF2 at all still see everything (any sound
  // beats silence).
  const grouped = useMemo(() => {
    const q = query.trim().toLowerCase();
    let instruments = plugins.filter((p) => p.isInstrument !== false);
    if (targetKind === 'default' && instruments.some((p) => p.format === 'Sf2')) {
      instruments = instruments.filter((p) => p.format === 'Sf2');
    }
    const filtered = q
      ? instruments.filter(
          (p) =>
            p.name.toLowerCase().includes(q) ||
            p.path.toLowerCase().includes(q) ||
            p.format.toLowerCase().includes(q),
        )
      : instruments;
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
  }, [plugins, query, targetKind]);

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
          <h3 className="text-sm font-medium text-[#e0e0e0]">选择音色</h3>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={handleRescan}
              disabled={scanning}
              className="text-[10px] px-2 py-1 rounded bg-daw-control hover:bg-daw-border text-[#aaa] transition-colors disabled:opacity-50"
              title="重新扫描插件目录"
            >
              {scanning ? '扫描中…' : '重新扫描'}
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
            placeholder="搜索音色…"
            autoFocus
            className="w-full bg-daw-control border border-daw-border rounded px-3 py-2 text-xs text-[#e0e0e0] placeholder-[#555] outline-none focus:border-daw-accent"
          />
        </div>

        {/* List */}
        <div className="flex-1 overflow-y-auto px-2 py-2 min-h-[180px]">
          {scanning && plugins.length === 0 && (
            <div className="text-xs text-[#666] text-center py-8">正在扫描插件目录…</div>
          )}
          {!scanning && plugins.length === 0 && (
            <div className="text-xs text-[#666] text-center py-8">
              没有找到任何音色。可以在下方直接粘贴文件路径加载。
            </div>
          )}
          {grouped.length === 0 && plugins.length > 0 && (
            <div className="text-xs text-[#666] text-center py-8">
              没有匹配 “{query}” 的音色。
            </div>
          )}
          {grouped.map((group) => {
            const base = FORMAT_META[group.format] ?? { label: group.format, badge: 'bg-[#444] text-[#aaa]' };
            const meta =
              targetKind === 'default' && FORMAT_META_DEFAULT_SLOT[group.format]
                ? { ...base, label: FORMAT_META_DEFAULT_SLOT[group.format] }
                : base;
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
            或者直接粘贴路径
          </div>
          <div className="flex gap-2">
            <input
              type="text"
              value={path}
              onChange={(e) => setPath(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') handleLoadFromPath();
              }}
              placeholder="/path/to/instrument.sf2  或  /Library/Audio/Plug-Ins/VST3/Foo.vst3"
              className="flex-1 bg-daw-control border border-daw-border rounded px-3 py-1.5 text-xs text-[#e0e0e0] placeholder-[#555] outline-none focus:border-daw-accent"
            />
            <button
              type="button"
              onClick={handleLoadFromPath}
              disabled={path.trim().length === 0}
              className="px-3 py-1.5 text-xs rounded bg-daw-accent hover:bg-daw-accent/80 text-white transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
            >
              加载
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
