import { useState, useCallback } from 'react';
import { useTransportStore } from '../stores/transport';
import { useSessionStore } from '../stores/session';

function formatPosition(ticks: number): string {
  const ticksPerBeat = 480;
  const beatsPerBar = 4;
  const ticksPerBar = ticksPerBeat * beatsPerBar;

  const bar = Math.floor(ticks / ticksPerBar) + 1;
  const beat = Math.floor((ticks % ticksPerBar) / ticksPerBeat) + 1;
  const tick = ticks % ticksPerBeat;

  return `${String(bar).padStart(3, '0')}:${beat}:${String(tick).padStart(3, '0')}`;
}

export function TransportBar() {
  const playing = useTransportStore((s) => s.playing);
  const bpm = useTransportStore((s) => s.bpm);
  const position = useTransportStore((s) => s.position);
  const timeSignature = useTransportStore((s) => s.timeSignature);
  const send = useSessionStore((s) => s.send);
  const connected = useSessionStore((s) => s.connected);

  const [editingBpm, setEditingBpm] = useState(false);
  const [bpmDraft, setBpmDraft] = useState('');

  const handlePlay = useCallback(() => {
    if (playing) {
      send({ type: 'transport.stop' });
    } else {
      send({ type: 'transport.play' });
    }
  }, [playing, send]);

  const handleStop = useCallback(() => {
    send({ type: 'transport.stop' });
  }, [send]);

  const handleBpmClick = useCallback(() => {
    setBpmDraft(bpm.toFixed(1));
    setEditingBpm(true);
  }, [bpm]);

  const handleBpmConfirm = useCallback(() => {
    const parsed = parseFloat(bpmDraft);
    if (!Number.isNaN(parsed) && parsed >= 20 && parsed <= 999) {
      send({ type: 'transport.set_bpm', bpm: parsed });
    }
    setEditingBpm(false);
  }, [bpmDraft, send]);

  const handleBpmKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Enter') {
        handleBpmConfirm();
      } else if (e.key === 'Escape') {
        setEditingBpm(false);
      }
    },
    [handleBpmConfirm],
  );

  return (
    <div className="h-12 bg-daw-panel border-b border-daw-border flex items-center px-4 gap-4 select-none">
      {/* Logo + connection */}
      <div className="flex items-center gap-2">
        <span className="text-daw-accent font-bold tracking-wide">moonlitt</span>
        <div className="flex items-center gap-1.5">
          <div className={`w-2 h-2 rounded-full ${connected ? 'bg-green-400' : 'bg-red-400'}`} />
          <span className="text-xs text-[#888]">{connected ? 'connected' : 'offline'}</span>
        </div>
      </div>

      {/* Divider */}
      <div className="w-px h-6 bg-daw-border" />

      {/* Transport controls */}
      <div className="flex items-center gap-1">
        {/* Rewind */}
        <button
          type="button"
          className="w-8 h-8 flex items-center justify-center rounded bg-daw-control hover:bg-daw-border text-[#e0e0e0] transition-colors"
          title="Rewind"
        >
          <span className="text-xs">&#9664;&#9664;</span>
        </button>

        {/* Stop */}
        <button
          type="button"
          onClick={handleStop}
          className="w-8 h-8 flex items-center justify-center rounded bg-daw-control hover:bg-daw-border text-[#e0e0e0] transition-colors"
          title="Stop"
        >
          <span className="text-sm">&#9632;</span>
        </button>

        {/* Play */}
        <button
          type="button"
          onClick={handlePlay}
          className={`w-8 h-8 flex items-center justify-center rounded transition-colors ${
            playing
              ? 'bg-daw-accent text-white'
              : 'bg-daw-control hover:bg-daw-border text-[#e0e0e0]'
          }`}
          title={playing ? 'Pause' : 'Play'}
        >
          <span className="text-sm">&#9654;</span>
        </button>

        {/* Record (disabled) */}
        <button
          type="button"
          disabled
          className="w-8 h-8 flex items-center justify-center rounded bg-daw-control text-[#555] cursor-not-allowed"
          title="Record (not available)"
        >
          <span className="text-sm">&#9679;</span>
        </button>
      </div>

      {/* Divider */}
      <div className="w-px h-6 bg-daw-border" />

      {/* Position display */}
      <div className="flex items-center gap-2">
        <span className="text-[#888] text-xs">POS</span>
        <span className="font-mono text-sm tracking-wider">{formatPosition(position)}</span>
      </div>

      {/* Divider */}
      <div className="w-px h-6 bg-daw-border" />

      {/* BPM display / editor */}
      <div className="flex items-center gap-2">
        <span className="text-[#888] text-xs">BPM</span>
        {editingBpm ? (
          <input
            type="text"
            value={bpmDraft}
            onChange={(e) => setBpmDraft(e.target.value)}
            onKeyDown={handleBpmKeyDown}
            onBlur={handleBpmConfirm}
            className="w-16 bg-daw-control border border-daw-accent rounded px-1 py-0.5 text-sm font-mono text-[#e0e0e0] outline-none"
            autoFocus
          />
        ) : (
          <button
            type="button"
            onClick={handleBpmClick}
            className="font-mono text-sm hover:text-daw-accent transition-colors cursor-pointer"
            title="Click to edit BPM"
          >
            {bpm.toFixed(1)}
          </button>
        )}
      </div>

      {/* Divider */}
      <div className="w-px h-6 bg-daw-border" />

      {/* Time signature */}
      <div className="flex items-center gap-2">
        <span className="text-[#888] text-xs">SIG</span>
        <span className="font-mono text-sm">{timeSignature[0]}/{timeSignature[1]}</span>
      </div>
    </div>
  );
}
