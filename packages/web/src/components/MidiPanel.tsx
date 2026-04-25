import { useCallback, useRef, useState } from 'react';
import type { MidiState } from '@moonlitt/protocol';
import { uploadMidiFile } from '../services/upload';

interface MidiPanelProps {
  midi: MidiState | null;
}

export function MidiPanel({ midi }: MidiPanelProps) {
  const [dragOver, setDragOver] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const upload = useCallback(async (file: File) => {
    if (!file.name.match(/\.midi?$/i)) {
      setError(`不是 MIDI 文件: ${file.name}`);
      return;
    }
    setError(null);
    setBusy(true);
    const ok = await uploadMidiFile(file, 0);
    setBusy(false);
    if (!ok) setError('上传失败（请查看服务端日志）');
  }, []);

  const dropProps = {
    onDragOver: (e: React.DragEvent) => { e.preventDefault(); setDragOver(true); },
    onDragLeave: () => setDragOver(false),
    onDrop: async (e: React.DragEvent) => {
      e.preventDefault();
      setDragOver(false);
      const file = e.dataTransfer.files[0];
      if (file) await upload(file);
    },
    onClick: () => fileInputRef.current?.click(),
  };

  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file) upload(file);
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
          <div className="text-4xl mb-3">📁</div>
          <div className="text-base text-[#e0e0e0] mb-1">
            {busy ? '上传中…' : '拖一个 .mid 文件到这里开始'}
          </div>
          <div className="text-xs text-[#888]">
            {busy ? '' : '或者点击此区域选择文件'}
          </div>
          <div className="text-[10px] text-[#555] mt-3">
            通道会从文件解析出来，每个通道一行
          </div>
        </div>
        <input
          ref={fileInputRef}
          type="file"
          accept=".mid,.midi"
          onChange={handleFileChange}
          className="hidden"
        />
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
        <span>📁</span>
        <div className="flex-1 min-w-0">
          <span className="text-[#e0e0e0] font-medium">{midi.name}</span>
          <span className="text-[#666] ml-3">
            {midi.channels.length} 通道 · {bpm} BPM · {ts} · {Math.round(midi.lengthBars)} 小节
          </span>
        </div>
        <span className="text-[10px] text-daw-accent/80">
          {busy ? '上传中…' : '更换…'}
        </span>
      </div>
      <input
        ref={fileInputRef}
        type="file"
        accept=".mid,.midi"
        onChange={handleFileChange}
        className="hidden"
      />
      {error !== null && <div className="mt-1 text-[11px] text-red-400">{error}</div>}
    </div>
  );
}
