import { useState } from 'react';
import { useProjectFileStore } from '../stores/projectFile';
import { useProjectStore } from '../stores/project';
import { useSessionStore } from '../stores/session';
import { ProjectMenu } from './ProjectMenu';
import { Meter } from './Meter';

interface TopBarProps {
  connected: boolean;
  playing: boolean;
  looping: boolean;
  metronomeEnabled: boolean;
  position: number;
  bpm: number;
  onPlay(): void;
  onStop(): void;
  onLoopToggle(): void;
  onMetronomeToggle(): void;
  onBpmChange(bpm: number): void;
}

function formatDb(db: number): string {
  if (db <= -59.5) return '-∞';
  return `${db >= 0 ? '+' : ''}${db.toFixed(1)}`;
}

function formatBarsBeats(ticks: number): string {
  const tpq = 480;
  const beatsPerBar = 4;
  const ticksPerBar = tpq * beatsPerBar;
  const bar = Math.floor(ticks / ticksPerBar) + 1;
  const beat = Math.floor((ticks % ticksPerBar) / tpq) + 1;
  return `${bar}.${beat}`;
}

function formatSmpte(ticks: number, bpm: number): string {
  // Assumes constant tempo — fine for the testbed, but a MIDI with
  // tempo changes will read slightly off. The engine-side play head
  // could feed real elapsed seconds back later if it matters.
  const tpq = 480;
  const seconds = (ticks / tpq) * (60 / bpm);
  const total = Math.max(0, seconds);
  const m = Math.floor(total / 60);
  const s = Math.floor(total % 60);
  const ms = Math.floor((total - Math.floor(total)) * 1000);
  return `${m.toString().padStart(2, '0')}:${s.toString().padStart(2, '0')}.${ms.toString().padStart(3, '0')}`;
}

export function TopBar({
  connected,
  playing,
  looping,
  metronomeEnabled,
  position,
  bpm,
  onPlay,
  onStop,
  onLoopToggle,
  onMetronomeToggle,
  onBpmChange,
}: TopBarProps) {
  const [editingBpm, setEditingBpm] = useState(false);
  const [draft, setDraft] = useState('');
  const currentPath = useProjectFileStore((s) => s.path);
  const dirty = useProjectFileStore((s) => s.dirty);
  const masterDb = useProjectStore((s) => s.masterVolumeDb);
  const setLocalMasterDb = useProjectStore((s) => s.setMasterVolume);
  const send = useSessionStore((s) => s.send);

  const onMasterChange = (db: number) => {
    setLocalMasterDb(db);
    send({ type: 'master.set_volume', db });
  };

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
            className={`w-20 h-8 rounded text-sm font-semibold border transition-colors disabled:opacity-30 ${
              playing
                ? 'bg-daw-accent border-daw-accent text-white shadow-[inset_0_1px_0_rgba(255,255,255,0.12)]'
                : 'bg-daw-control border-daw-border hover:border-[#555] text-[#e0e0e0]'
            }`}
            title="空格切换播放"
          >
            {playing ? '❚❚ 暂停' : '▶ 播放'}
          </button>
          <button
            type="button"
            onClick={onStop}
            disabled={!connected}
            className="w-16 h-8 rounded border border-daw-border bg-daw-control hover:border-[#555] text-[#e0e0e0] text-sm transition-colors disabled:opacity-30"
          >
            ■ 停止
          </button>
          <button
            type="button"
            onClick={onLoopToggle}
            disabled={!connected}
            className={`w-10 h-8 rounded border text-sm transition-colors disabled:opacity-30 ${
              looping
                ? 'bg-daw-accent border-daw-accent text-white'
                : 'bg-daw-control border-daw-border hover:border-[#555] text-[#aaa]'
            }`}
            title={looping ? '循环开（点击关闭）' : '循环关（点击开启）'}
          >
            ↺
          </button>
          <button
            type="button"
            onClick={onMetronomeToggle}
            disabled={!connected}
            className={`w-10 h-8 rounded border text-sm transition-colors disabled:opacity-30 ${
              metronomeEnabled
                ? 'bg-daw-accent border-daw-accent text-white'
                : 'bg-daw-control border-daw-border hover:border-[#555] text-[#aaa]'
            }`}
            title={metronomeEnabled ? '节拍器开（点击关闭）' : '节拍器关（点击开启）'}
          >
            ♩
          </button>

          {/* Master section — meter on top, fader below. Slightly taller
              than the transport bay so it stands out the way a master
              channel strip does in Logic / Bitwig / Studio One. */}
          <div className="ml-2 flex flex-col justify-center h-11 px-2 py-1 rounded border border-daw-border bg-daw-surface gap-1">
            <Meter channel={null} width={92} height={4} />
            <div className="flex items-center gap-1.5">
              <input
                type="range"
                min={-60}
                max={6}
                step={0.5}
                value={masterDb}
                onChange={(e) => onMasterChange(parseFloat(e.target.value))}
                onDoubleClick={() => onMasterChange(0)}
                className="w-14 h-0.5 accent-daw-accent"
                title="Master fader (双击恢复 0 dB)"
              />
              <span className="text-[9px] font-mono tabular-nums text-[#aaa] w-9 text-right leading-none">
                {formatDb(masterDb)}
              </span>
            </div>
          </div>

          {/* Transport readout — bay-style block, mimics Logic/Bitwig
              transport modules. Bigger tabular digits, internal divider,
              tiny uppercase labels. */}
          <div className="flex items-stretch h-8 rounded border border-daw-border bg-daw-surface overflow-hidden">
            <div className="flex flex-col justify-center px-3 min-w-[64px]">
              <span className="text-[8px] uppercase tracking-[0.12em] text-[#666] leading-none">位置</span>
              <span className="font-mono text-sm tabular-nums text-[#e0e0e0] leading-tight mt-0.5">
                {formatBarsBeats(position)}
              </span>
            </div>
            <div className="w-px bg-daw-border" />
            <div className="flex flex-col justify-center px-3 min-w-[92px]">
              <span className="text-[8px] uppercase tracking-[0.12em] text-[#666] leading-none">时码</span>
              <span className="font-mono text-sm tabular-nums text-[#e0e0e0] leading-tight mt-0.5">
                {formatSmpte(position, bpm)}
              </span>
            </div>
            <div className="w-px bg-daw-border" />
            <div className="flex flex-col justify-center px-3 min-w-[64px]">
              <span className="text-[8px] uppercase tracking-[0.12em] text-[#666] leading-none">节拍</span>
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
                  className="w-12 bg-transparent border-b border-daw-accent text-sm font-mono tabular-nums text-[#e0e0e0] outline-none mt-0.5"
                />
              ) : (
                <button
                  type="button"
                  onClick={() => { setDraft(bpm.toFixed(1)); setEditingBpm(true); }}
                  className="font-mono text-sm tabular-nums text-[#e0e0e0] leading-tight mt-0.5 text-left hover:text-daw-accent cursor-pointer"
                  title="点击修改节拍"
                >
                  {bpm.toFixed(1)}
                </button>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
