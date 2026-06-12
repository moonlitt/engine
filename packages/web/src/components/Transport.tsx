import { useEffect, useRef, useState } from 'react';
import { useProjectFileStore } from '../stores/projectFile';
import { useProjectStore } from '../stores/project';
import { useSessionStore } from '../stores/session';
import { useTransportStore } from '../stores/transport';
import { useMetersStore } from '../stores/meters';
import { ProjectMenu } from './ProjectMenu';
import { formatBarsBeats, formatClock } from '../lib/time';

/**
 * The transport bar — hardware-grade chrome with a phosphor LCD block
 * in the middle, Logic-style: bars.beats as the hero readout, clock and
 * tempo beside it. The LCD playhead digits draw imperatively at meter
 * rate so the React tree stays still while playing.
 */
export function Transport() {
  const connected = useSessionStore((s) => s.connected);
  const send = useSessionStore((s) => s.send);
  const playing = useTransportStore((s) => s.playing);
  const looping = useTransportStore((s) => s.looping);
  const metronomeEnabled = useTransportStore((s) => s.metronomeEnabled);
  const bpm = useTransportStore((s) => s.bpm);
  const midi = useProjectStore((s) => s.midi);
  const currentPath = useProjectFileStore((s) => s.path);
  const dirty = useProjectFileStore((s) => s.dirty);
  const masterDb = useProjectStore((s) => s.masterVolumeDb);
  const setLocalMasterDb = useProjectStore((s) => s.setMasterVolume);

  return (
    <header className="shrink-0 bg-daw-panel border-b border-black/50 shadow-strip">
      <div className="flex items-stretch gap-3 px-4 py-2.5">
        {/* Project chip + connection lamp */}
        <div className="flex items-center gap-2.5 min-w-0">
          <ProjectMenu currentPath={currentPath} dirty={dirty} />
          <div
            className={`w-1.5 h-1.5 rounded-full ${connected ? 'bg-meter-green' : 'bg-meter-red'}`}
            title={connected ? '音频引擎已连接' : '音频引擎未连接'}
          />
        </div>

        {/* Transport keys */}
        <div className="flex items-center gap-1.5 ml-2">
          <button
            type="button"
            className="t-btn"
            title="回到开头"
            disabled={!midi}
            onClick={() => send({ type: 'transport.seek', ticks: 0 })}
          >
            <RewindIcon />
          </button>
          <button
            type="button"
            className="t-btn w-12"
            data-active={playing}
            data-flavor="play"
            title={playing ? '暂停（位置保留）' : '播放'}
            disabled={!midi}
            onClick={() => send({ type: playing ? 'transport.pause' : 'transport.play' })}
          >
            {playing ? <PauseIcon /> : <PlayIcon />}
          </button>
          <button
            type="button"
            className="t-btn"
            title="停止（回到开头）"
            disabled={!midi}
            onClick={() => send({ type: 'transport.stop' })}
          >
            <StopIcon />
          </button>
          <div className="w-px self-stretch my-1.5 bg-daw-line mx-1" />
          <button
            type="button"
            className="t-btn"
            data-active={looping}
            title="循环播放"
            onClick={() => send({ type: 'transport.set_loop', looping: !looping })}
          >
            <LoopIcon />
          </button>
          <button
            type="button"
            className="t-btn"
            data-active={metronomeEnabled}
            title="节拍器"
            onClick={() => send({ type: 'transport.set_metronome', enabled: !metronomeEnabled })}
          >
            <MetronomeIcon />
          </button>
        </div>

        {/* The LCD */}
        <Lcd bpm={bpm} onBpmChange={(v) => send({ type: 'transport.set_bpm', bpm: v })} />

        {/* Master strip */}
        <div className="ml-auto flex items-center gap-3">
          <MasterMeter />
          <div className="flex flex-col justify-center gap-0.5 w-36">
            <div className="flex items-baseline justify-between">
              <span className="lcd-label text-[#7c776c]">Master</span>
              <span className="font-lcd text-[10px] text-[#a59f93] tabular-nums">
                {masterDb <= -59.5 ? '-∞' : `${masterDb >= 0 ? '+' : ''}${masterDb.toFixed(1)}`} dB
              </span>
            </div>
            <input
              type="range"
              className="fader w-full"
              min={-60}
              max={6}
              step={0.5}
              value={masterDb}
              onChange={(e) => {
                const db = parseFloat(e.target.value);
                setLocalMasterDb(db);
                send({ type: 'master.set_volume', db });
              }}
            />
          </div>
        </div>
      </div>
    </header>
  );
}

/** Phosphor readout block. Position digits update imperatively. */
function Lcd({ bpm, onBpmChange }: { bpm: number; onBpmChange(v: number): void }) {
  const midi = useProjectStore((s) => s.midi);
  const [editingBpm, setEditingBpm] = useState(false);
  const [draft, setDraft] = useState('');
  const barsRef = useRef<HTMLSpanElement>(null);
  const clockRef = useRef<HTMLSpanElement>(null);

  // Imperative playhead digits at meter rate — no React re-renders.
  useEffect(() => {
    if (!midi) return;
    const draw = (ticks: number) => {
      if (barsRef.current) barsRef.current.textContent = formatBarsBeats(ticks, midi);
      if (clockRef.current) clockRef.current.textContent = formatClock(ticks, midi);
    };
    draw(useMetersStore.getState().playheadTicks);
    return useMetersStore.subscribe((s) => draw(s.playheadTicks));
  }, [midi]);

  const [num, den] = midi?.timeSignature ?? [4, 4];
  const fileTempo = midi?.tempoBpm ?? null;

  return (
    <div className="lcd flex items-stretch px-4 py-1.5 gap-5 min-w-[21rem]">
      {/* Position */}
      <div className="flex flex-col justify-center min-w-[6.5rem]">
        <span ref={barsRef} className="lcd-digits text-[1.55rem] leading-7">
          1.1
        </span>
        <span ref={clockRef} className="lcd-digits text-[10px] opacity-75">
          00:00.000
        </span>
      </div>
      <div className="w-px bg-black/40 my-1" />
      {/* Tempo */}
      <div className="flex flex-col justify-center cursor-text" onClick={() => {
        setDraft(String(Math.round(bpm * 10) / 10));
        setEditingBpm(true);
      }}>
        <span className="lcd-label">Tempo</span>
        {editingBpm ? (
          <input
            autoFocus
            className="lcd-digits bg-transparent outline-none w-16 text-lg border-b border-lcd-dim/50"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={() => setEditingBpm(false)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                const v = parseFloat(draft);
                if (Number.isFinite(v) && v >= 20 && v <= 400) onBpmChange(v);
                setEditingBpm(false);
              }
              if (e.key === 'Escape') setEditingBpm(false);
            }}
          />
        ) : (
          <span className="lcd-digits text-lg leading-6" title="点击输入固定曲速（覆盖文件曲速图）">
            {(Math.round(bpm * 10) / 10).toFixed(1)}
          </span>
        )}
        <span className="lcd-label normal-case tracking-normal text-[9px]">
          {fileTempo !== null ? '跟随文件曲速' : 'bpm'}
        </span>
      </div>
      <div className="w-px bg-black/40 my-1" />
      {/* Time signature + clip */}
      <div className="flex flex-col justify-center">
        <span className="lcd-label">拍号</span>
        <span className="lcd-digits text-lg leading-6">{num}/{den}</span>
        <span className="lcd-label normal-case tracking-normal text-[9px] max-w-[9rem] truncate">
          {midi ? midi.name : '未载入 MIDI'}
        </span>
      </div>
    </div>
  );
}

/** Master stereo meter — canvas-free, two divs driven imperatively. */
function MasterMeter() {
  const lRef = useRef<HTMLDivElement>(null);
  const rRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const draw = (l: number, r: number) => {
      if (lRef.current) lRef.current.style.height = `${Math.min(100, l * 100)}%`;
      if (rRef.current) rRef.current.style.height = `${Math.min(100, r * 100)}%`;
    };
    const s = useMetersStore.getState();
    draw(s.master.l, s.master.r);
    return useMetersStore.subscribe((s2) => draw(s2.master.l, s2.master.r));
  }, []);
  return (
    <div className="flex gap-[3px] h-10 items-end" title="主输出电平">
      <div className="meter-track w-[5px] h-full relative">
        <div ref={lRef} className="meter-fill absolute bottom-0 left-0 right-0" style={{ height: 0 }} />
      </div>
      <div className="meter-track w-[5px] h-full relative">
        <div ref={rRef} className="meter-fill absolute bottom-0 left-0 right-0" style={{ height: 0 }} />
      </div>
    </div>
  );
}

// --- Transport glyphs (inline SVG keeps them crisp at any DPI) -----------

function PlayIcon() {
  return (
    <svg width="11" height="12" viewBox="0 0 11 12" fill="currentColor">
      <path d="M0.5 0.8 L10.5 6 L0.5 11.2 Z" />
    </svg>
  );
}
function PauseIcon() {
  return (
    <svg width="10" height="12" viewBox="0 0 10 12" fill="currentColor">
      <rect x="0" y="0" width="3.4" height="12" rx="0.8" />
      <rect x="6.6" y="0" width="3.4" height="12" rx="0.8" />
    </svg>
  );
}
function StopIcon() {
  return (
    <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor">
      <rect width="10" height="10" rx="1.2" />
    </svg>
  );
}
function RewindIcon() {
  return (
    <svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor">
      <rect x="0" y="1" width="2" height="10" rx="0.6" />
      <path d="M11 1.5 L3.5 6 L11 10.5 Z" />
    </svg>
  );
}
function LoopIcon() {
  return (
    <svg width="13" height="12" viewBox="0 0 13 12" fill="none" stroke="currentColor" strokeWidth="1.4">
      <path d="M3.5 4 H9.5 A2.5 2.5 0 0 1 12 6.5 V6.5 A2.5 2.5 0 0 1 9.5 9 H3.5 A2.5 2.5 0 0 1 1 6.5 V6.5 A2.5 2.5 0 0 1 3.5 4 Z" opacity="0" />
      <path d="M4 2.5 H9 A3 3 0 0 1 12 5.5 A3 3 0 0 1 9 8.5 H4 A3 3 0 0 1 1 5.5 A3 3 0 0 1 4 2.5" />
      <path d="M4 0.8 L2.2 2.5 L4 4.2" fill="none" />
    </svg>
  );
}
function MetronomeIcon() {
  return (
    <svg width="11" height="12" viewBox="0 0 11 12" fill="none" stroke="currentColor" strokeWidth="1.3">
      <path d="M4 1 H7 L9.5 11 H1.5 Z" />
      <path d="M5.5 8 L8.5 2.5" />
    </svg>
  );
}
