import { useCallback, useRef, useState } from 'react';
import type { MidiState } from '@moonlitt/protocol';
import { getTransport } from '../services/transport';

interface MidiPanelProps {
  midi: MidiState | null;
}

export function MidiPanel({ midi }: MidiPanelProps) {
  const transport = getTransport();
  const [dragOver, setDragOver] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const loadFromFile = useCallback(async (file: File) => {
    if (!file.name.match(/\.midi?$/i)) {
      setError(`不是 MIDI 文件: ${file.name}`);
      return;
    }
    setError(null);
    setBusy(true);
    try {
      const ok = await transport.loadMidiFile(file);
      if (!ok) setError('加载失败（请查看控制台日志）');
    } catch (e) {
      setError(`加载异常: ${(e as Error)?.message ?? String(e)}`);
    } finally {
      setBusy(false);
    }
  }, [transport]);

  const openPicker = useCallback(async () => {
    if (transport.supportsFileDrop) {
      fileInputRef.current?.click();
      return;
    }
    setError(null);
    setBusy(true);
    try {
      const ok = await transport.pickAndLoadMidi();
      if (!ok) setError('未选择文件，或加载失败。');
    } catch (e) {
      setError(`选择 / 加载失败: ${(e as Error)?.message ?? String(e)}`);
    } finally {
      setBusy(false);
    }
  }, [transport]);

  const supportsDrop = transport.supportsFileDrop;
  const dropProps = supportsDrop
    ? {
        onDragOver: (e: React.DragEvent) => { e.preventDefault(); setDragOver(true); },
        onDragLeave: () => setDragOver(false),
        onDrop: async (e: React.DragEvent) => {
          e.preventDefault();
          setDragOver(false);
          const file = e.dataTransfer.files[0];
          if (file) await loadFromFile(file);
        },
        onClick: () => openPicker(),
      }
    : {
        onClick: () => openPicker(),
      };

  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file) void loadFromFile(file);
    e.target.value = '';
  };

  if (midi === null) {
    return (
      <div>
        <div
          {...dropProps}
          className={`cursor-pointer rounded-xl border-2 border-dashed px-8 py-16 text-center transition-colors ${
            dragOver
              ? 'border-daw-accent bg-daw-accent/10'
              : 'border-daw-border hover:border-daw-accent/60 hover:bg-daw-control/30'
          }`}
        >
          <MidiDinIcon className={`mx-auto mb-4 transition-colors ${dragOver ? 'text-daw-accent' : 'text-[#5a564e]'}`} />
          <div className="text-base text-[#e0e0e0] mb-1">
            {busy
              ? '加载中…'
              : supportsDrop
                ? '拖一个 .mid 文件到这里开始'
                : '点击此区域选择 MIDI 文件'}
          </div>
          <div className="text-xs text-[#8a857b]">
            {busy ? '' : supportsDrop ? '或者点击此区域选择文件' : ''}
          </div>
          <div className="lcd-label !text-[9px] text-[#5a564e] mt-4">
            通道会从文件解析出来 · 每个通道一行
          </div>
        </div>
        {supportsDrop && (
          <input
            ref={fileInputRef}
            type="file"
            accept=".mid,.midi"
            onChange={handleFileChange}
            className="hidden"
          />
        )}
        {error !== null && <div className="mt-3 text-xs text-red-400 text-center">{error}</div>}
      </div>
    );
  }

  const ts = midi.timeSignature ? `${midi.timeSignature[0]}/${midi.timeSignature[1]}` : '?';
  const bpm = midi.tempoBpm !== null ? midi.tempoBpm.toFixed(0) : '?';

  return (
    <div>
      <div
        {...dropProps}
        className={`cursor-pointer flex items-center gap-3 rounded border border-dashed px-4 py-2.5 text-xs transition-colors ${
          dragOver
            ? 'border-daw-accent bg-daw-accent/10'
            : 'border-daw-border hover:border-daw-accent/60 hover:bg-daw-control/30'
        }`}
      >
        <span className="lcd-label !text-[9px] text-[#7c776c] border border-daw-line rounded px-1.5 py-0.5">
          MIDI
        </span>
        <div className="flex-1 min-w-0">
          <span className="text-[#e0e0e0] font-medium">{midi.name}</span>
          <span className="text-[#6b675f] ml-3 font-lcd text-[10px] tabular-nums">
            {midi.channels.length} 通道 · {bpm} BPM · {ts} · {Math.round(midi.lengthBars)} 小节
          </span>
        </div>
        <span className="text-[10px] text-daw-accent/80">
          {busy ? '加载中…' : '更换…'}
        </span>
      </div>
      {supportsDrop && (
        <input
          ref={fileInputRef}
          type="file"
          accept=".mid,.midi"
          onChange={handleFileChange}
          className="hidden"
        />
      )}
      {error !== null && <div className="mt-1 text-[11px] text-red-400">{error}</div>}
    </div>
  );
}

/** MIDI DIN-5 connector glyph — pins fanned in the classic 180° arc. */
function MidiDinIcon({ className }: { className?: string }) {
  return (
    <svg
      width="44"
      height="44"
      viewBox="0 0 44 44"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      className={className}
      aria-hidden
    >
      <circle cx="22" cy="22" r="18" />
      <rect x="19.5" y="4.5" width="5" height="3.5" rx="1" fill="currentColor" stroke="none" />
      {[
        [22, 31],
        [14.5, 28],
        [29.5, 28],
        [11, 20.5],
        [33, 20.5],
      ].map(([x, y]) => (
        <circle key={`${x}-${y}`} cx={x} cy={y} r="1.9" fill="currentColor" stroke="none" />
      ))}
    </svg>
  );
}
