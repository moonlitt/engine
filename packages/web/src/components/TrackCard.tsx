import { useCallback, useState } from 'react';
import type { ParamMeta } from '@moonlitt/protocol';
import { useMixerStore, type Insert, type Track } from '../stores/mixer';
import { useSessionStore } from '../stores/session';
import { useUiStore } from '../stores/ui';

const EFFECT_TYPES: readonly { label: string; value: string }[] = [
  { label: 'Dattorro Reverb', value: 'dattorro-reverb' },
  { label: 'Reverb (Freeverb)', value: 'reverb' },
  { label: 'Delay', value: 'delay' },
  { label: 'Chorus', value: 'chorus' },
  { label: 'Flanger', value: 'flanger' },
  { label: 'Phaser', value: 'phaser' },
  { label: 'Tremolo', value: 'tremolo' },
  { label: 'EQ', value: 'eq' },
  { label: 'Compressor', value: 'compressor' },
  { label: 'Limiter', value: 'limiter' },
  { label: 'Gate', value: 'gate' },
  { label: 'De-esser', value: 'de-esser' },
  { label: 'Multiband Compressor', value: 'multiband-compressor' },
  { label: 'Saturator', value: 'saturator' },
  { label: 'Bitcrusher', value: 'bitcrusher' },
  { label: 'Auto Filter', value: 'auto-filter' },
  { label: 'Pitch Shifter', value: 'pitch-shifter' },
  { label: 'Stereo Width', value: 'stereo-width' },
  { label: 'Gain', value: 'gain' },
];

export function TrackCard({ track }: { track: Track }) {
  const send = useSessionStore((s) => s.send);
  const setTrackMute = useMixerStore((s) => s.setTrackMute);
  const setTrackSolo = useMixerStore((s) => s.setTrackSolo);
  const setTrackVolume = useMixerStore((s) => s.setTrackVolume);
  const openInstrumentSelector = useUiStore((s) => s.openInstrumentSelector);

  const [effectsOpen, setEffectsOpen] = useState(true);

  const handleMute = useCallback(() => {
    const muted = !track.muted;
    setTrackMute(track.id, muted);
    send({ type: 'track.set_mute', trackId: track.id, muted });
  }, [track.id, track.muted, setTrackMute, send]);

  const handleSolo = useCallback(() => {
    const solo = !track.solo;
    setTrackSolo(track.id, solo);
    send({ type: 'track.set_solo', trackId: track.id, solo });
  }, [track.id, track.solo, setTrackSolo, send]);

  const handleVolume = useCallback(
    (db: number) => {
      setTrackVolume(track.id, db);
      send({ type: 'track.set_volume', trackId: track.id, db });
    },
    [track.id, setTrackVolume, send],
  );

  const instrumentLabel = track.instrumentName ?? track.instrumentPath?.split('/').pop() ?? null;

  return (
    <section className="bg-daw-panel border border-daw-border rounded-lg overflow-hidden">
      {/* Top strip: color + name + instrument + M/S + volume */}
      <div className="flex items-center gap-3 px-3 py-2 border-b border-daw-border">
        <div className="w-1 self-stretch rounded-sm" style={{ backgroundColor: track.color }} />
        <span className="text-sm font-medium text-[#e0e0e0] w-[80px] truncate" title={track.name}>
          {track.name}
        </span>

        <button
          type="button"
          onClick={() => openInstrumentSelector(track.id)}
          className={`flex-1 min-w-0 text-left px-3 py-1.5 rounded text-xs font-medium transition-colors ${
            instrumentLabel
              ? 'bg-daw-control hover:bg-daw-border text-[#e0e0e0]'
              : 'bg-daw-accent/20 hover:bg-daw-accent text-daw-accent hover:text-white border border-daw-accent/40'
          }`}
          title={track.instrumentPath ?? 'Click to choose an SF2 / VST3 / CLAP instrument'}
        >
          {instrumentLabel ? (
            <>
              <span className="opacity-60 mr-2">🎹</span>
              <span className="truncate inline-block max-w-[300px] align-middle">{instrumentLabel}</span>
              <span className="text-[#888] ml-2">change…</span>
            </>
          ) : (
            <>🎹 Pick instrument…</>
          )}
        </button>

        <ToggleBtn label="M" active={track.muted} activeClass="bg-red-500/80 text-white" onClick={handleMute} />
        <ToggleBtn label="S" active={track.solo} activeClass="bg-yellow-500/80 text-black" onClick={handleSolo} />

        <div className="flex items-center gap-2 w-[160px]">
          <input
            type="range"
            min={-60}
            max={6}
            step={0.5}
            value={track.volume}
            onChange={(e) => handleVolume(parseFloat(e.target.value))}
            className="flex-1 accent-daw-accent"
            title={`Volume: ${track.volume.toFixed(1)} dB`}
          />
          <span className="text-[10px] text-[#888] font-mono w-10 text-right tabular-nums">
            {track.volume.toFixed(1)}
          </span>
        </div>
      </div>

      {/* Clip indicator (read-only — MIDI upload lives in the global bar above) */}
      <ClipIndicator track={track} />

      {/* Effects */}
      <div className="border-t border-daw-border">
        <button
          type="button"
          onClick={() => setEffectsOpen((v) => !v)}
          className="w-full flex items-center gap-2 px-3 py-2 text-[10px] uppercase tracking-widest text-[#888] hover:text-[#ccc] transition-colors"
        >
          <span>{effectsOpen ? '▾' : '▸'}</span>
          <span>Effects ({track.inserts.length})</span>
        </button>
        {effectsOpen && (
          <div className="px-3 pb-3 flex flex-col gap-2">
            {track.inserts.map((insert) => (
              <InsertRow key={insert.id} trackId={track.id} insert={insert} />
            ))}
            <AddEffect onAdd={(fx) => send({ type: 'insert.add', trackId: track.id, effectType: fx })} />
          </div>
        )}
      </div>
    </section>
  );
}

// ---------------------------------------------------------------------------

function ToggleBtn({
  label, active, activeClass, onClick,
}: { label: string; active: boolean; activeClass: string; onClick(): void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`w-6 h-6 text-[10px] font-bold rounded transition-colors ${
        active ? activeClass : 'bg-daw-bg text-[#666] hover:text-white'
      }`}
    >
      {label}
    </button>
  );
}

// ---------------------------------------------------------------------------

function ClipIndicator({ track }: { track: Track }) {
  const clip = track.clips[0] ?? null;
  if (clip === null) {
    return (
      <div className="px-3 py-1.5 text-[10px] text-[#666]">
        No notes routed to this track yet — upload a MIDI in the bar above.
      </div>
    );
  }
  return (
    <div className="px-3 py-1.5 flex items-center gap-2 text-[10px] text-[#888]">
      <span className="text-green-400">▸</span>
      <span className="truncate">Plays <span className="text-[#bbb]">{clip.name}</span></span>
    </div>
  );
}

// ---------------------------------------------------------------------------

function AddEffect({ onAdd }: { onAdd(fx: string): void }) {
  const [picking, setPicking] = useState(false);
  if (!picking) {
    return (
      <button
        type="button"
        onClick={() => setPicking(true)}
        className="self-start px-2.5 py-1 rounded bg-daw-control hover:bg-daw-border text-[#ccc] text-[11px] font-medium transition-colors"
      >
        + Add effect…
      </button>
    );
  }
  return (
    <select
      autoFocus
      defaultValue=""
      onChange={(e) => { if (e.target.value) onAdd(e.target.value); setPicking(false); }}
      onBlur={() => setPicking(false)}
      className="self-start bg-daw-control border border-daw-accent rounded px-2 py-1 text-xs text-[#e0e0e0] outline-none"
    >
      <option value="" disabled>Select an effect…</option>
      {EFFECT_TYPES.map((fx) => (
        <option key={fx.value} value={fx.value}>{fx.label}</option>
      ))}
    </select>
  );
}

function InsertRow({ trackId, insert }: { trackId: number; insert: Insert }) {
  const send = useSessionStore((s) => s.send);
  const setInsertParam = useMixerStore((s) => s.setInsertParam);
  const removeInsertLocal = useMixerStore((s) => s.removeInsert);
  const [open, setOpen] = useState(true);

  const handleParam = useCallback(
    (param: ParamMeta, value: number) => {
      setInsertParam(trackId, insert.id, param.id, value);
      send({ type: 'insert.set_param', trackId, insertId: insert.id, paramId: param.id, value });
    },
    [trackId, insert.id, setInsertParam, send],
  );

  const handleRemove = useCallback(() => {
    removeInsertLocal(trackId, insert.id);
    send({ type: 'insert.remove', trackId, insertId: insert.id });
  }, [trackId, insert.id, removeInsertLocal, send]);

  return (
    <div className="rounded border border-daw-border bg-daw-control/40">
      <div className="flex items-center gap-2 px-2.5 py-1.5">
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="w-3 text-[10px] text-[#888] hover:text-[#ccc]"
          aria-label={open ? 'Collapse' : 'Expand'}
        >
          {open ? '▾' : '▸'}
        </button>
        <span className="w-1.5 h-1.5 rounded-full bg-green-400 shrink-0" />
        <span className="text-xs text-[#e0e0e0] flex-1 truncate">{insert.name}</span>
        <button
          type="button"
          onClick={handleRemove}
          className="text-[#666] hover:text-red-400 transition-colors px-1"
          title="Remove"
        >
          ×
        </button>
      </div>
      {open && insert.params.length > 0 && (
        <div className="px-2.5 pb-2.5 pt-0.5 flex flex-col gap-1.5 border-t border-daw-border">
          {insert.params.map((p) => (
            <ParamSlider key={p.id} param={p} onChange={(v) => handleParam(p, v)} />
          ))}
        </div>
      )}
    </div>
  );
}

function ParamSlider({ param, onChange }: { param: ParamMeta; onChange: (v: number) => void }) {
  const range = param.max - param.min;
  const step = param.stepCount > 0 ? range / param.stepCount : range / 1000;
  return (
    <div className="grid grid-cols-[7rem_1fr_3.5rem] items-center gap-2">
      <label className="text-[10px] text-[#aaa] truncate" title={param.name}>{param.name}</label>
      <input
        type="range"
        min={param.min}
        max={param.max}
        step={step}
        value={param.value}
        onChange={(e) => onChange(parseFloat(e.target.value))}
        className="w-full accent-daw-accent"
      />
      <span className="text-[10px] text-[#888] text-right tabular-nums">{formatValue(param.value, param)}</span>
    </div>
  );
}

function formatValue(value: number, param: ParamMeta): string {
  if (param.min === 0 && param.max === 1) return `${Math.round(value * 100)}%`;
  if (param.stepCount > 0 && param.stepCount <= 1) return value >= 0.5 ? 'on' : 'off';
  const abs = Math.abs(value);
  if (abs >= 1000) return value.toFixed(0);
  if (abs >= 100) return value.toFixed(1);
  if (abs >= 10) return value.toFixed(2);
  return value.toFixed(3);
}
