import { useState } from 'react';
import { useProjectFileStore } from '../stores/projectFile';
import { ProjectMenu } from './ProjectMenu';

interface TopBarProps {
  connected: boolean;
  playing: boolean;
  position: number;
  bpm: number;
  onPlay(): void;
  onStop(): void;
  onBpmChange(bpm: number): void;
}

function formatBarsBeats(ticks: number): string {
  const tpq = 480;
  const beatsPerBar = 4;
  const ticksPerBar = tpq * beatsPerBar;
  const bar = Math.floor(ticks / ticksPerBar) + 1;
  const beat = Math.floor((ticks % ticksPerBar) / tpq) + 1;
  return `${bar}.${beat}`;
}

export function TopBar({ connected, playing, position, bpm, onPlay, onStop, onBpmChange }: TopBarProps) {
  const [editingBpm, setEditingBpm] = useState(false);
  const [draft, setDraft] = useState('');
  const currentPath = useProjectFileStore((s) => s.path);
  const dirty = useProjectFileStore((s) => s.dirty);

  return (
    <div className="sticky top-0 z-10 bg-daw-panel border-b border-daw-border">
      <div className="max-w-[840px] mx-auto px-6 py-3 flex items-center gap-4">
        <div className="flex items-center gap-2">
          <span className="text-base font-semibold tracking-wide">
            moonlitt <span className="text-[#666] font-normal">player</span>
          </span>
          <div className="flex items-center gap-1.5 ml-2">
            <div className={`w-2 h-2 rounded-full ${connected ? 'bg-green-400' : 'bg-red-400'}`} />
            <span className="text-[10px] text-[#888]">{connected ? '已连接' : '未连接'}</span>
          </div>
        </div>

        <ProjectMenu currentPath={currentPath} dirty={dirty} />

        <div className="ml-auto flex items-center gap-2">
          <button
            type="button"
            onClick={onPlay}
            disabled={!connected}
            className={`px-4 py-1.5 rounded text-sm font-semibold transition-colors disabled:opacity-30 ${
              playing ? 'bg-daw-accent text-white' : 'bg-daw-control hover:bg-daw-border text-[#e0e0e0]'
            }`}
            title="空格切换播放"
          >
            {playing ? '❚❚ 暂停' : '▶ 播放'}
          </button>
          <button
            type="button"
            onClick={onStop}
            disabled={!connected}
            className="px-3 py-1.5 rounded bg-daw-control hover:bg-daw-border text-[#e0e0e0] text-sm transition-colors disabled:opacity-30"
          >
            ■ 停止
          </button>

          <div className="ml-3 flex items-center gap-3 font-mono text-xs text-[#aaa]">
            <span>
              <span className="text-[9px] text-[#666] mr-1">位置</span>
              {formatBarsBeats(position)}
            </span>
            {editingBpm ? (
              <input
                type="text"
                value={draft}
                autoFocus
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    const parsed = parseFloat(draft);
                    if (!Number.isNaN(parsed) && parsed >= 20 && parsed <= 999) onBpmChange(parsed);
                    setEditingBpm(false);
                  } else if (e.key === 'Escape') {
                    setEditingBpm(false);
                  }
                }}
                onBlur={() => {
                  const parsed = parseFloat(draft);
                  if (!Number.isNaN(parsed) && parsed >= 20 && parsed <= 999) onBpmChange(parsed);
                  setEditingBpm(false);
                }}
                className="w-16 bg-daw-control border border-daw-accent rounded px-1 py-0.5 text-xs font-mono outline-none"
              />
            ) : (
              <button
                type="button"
                onClick={() => { setDraft(bpm.toFixed(1)); setEditingBpm(true); }}
                className="hover:text-daw-accent cursor-pointer"
                title="点击修改节拍"
              >
                <span className="text-[9px] text-[#666] mr-1">节拍</span>
                {bpm.toFixed(1)}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
