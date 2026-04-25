import { useCallback, useEffect, useRef, useState } from 'react';
import type { ParamMeta } from '@moonlitt/protocol';
import { useMixerStore, type Insert, type Track } from '../stores/mixer';
import { useTransportStore } from '../stores/transport';
import { useSessionStore } from '../stores/session';
import { useUiStore } from '../stores/ui';
import { uploadMidiFile } from '../services/upload';

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

export function TestbedView() {
  const tracks = useMixerStore((s) => s.tracks);
  const send = useSessionStore((s) => s.send);
  const connected = useSessionStore((s) => s.connected);
  const playing = useTransportStore((s) => s.playing);
  const position = useTransportStore((s) => s.position);
  const bpm = useTransportStore((s) => s.bpm);
  const openInstrumentSelector = useUiStore((s) => s.openInstrumentSelector);

  // Auto-create the implicit single track once we're connected and have none.
  // Why: this UI presents itself as a single source/clip player, so a track
  // always existing matches the user's mental model.
  const autoCreatedRef = useRef(false);
  useEffect(() => {
    if (!connected || autoCreatedRef.current) return;
    if (tracks.length === 0) {
      autoCreatedRef.current = true;
      send({ type: 'track.add' });
    } else {
      autoCreatedRef.current = true;
    }
  }, [connected, tracks.length, send]);

  const track = tracks[0] ?? null;

  return (
    <div className="h-screen overflow-y-auto bg-daw-bg text-[#e0e0e0] font-sans">
      <div className="max-w-[680px] mx-auto py-8 px-6 flex flex-col gap-5">
        <Header connected={connected} />

        {track === null ? (
          <div className="text-center text-[#666] py-12">
            {connected ? 'Initializing track…' : 'Connecting to engine…'}
          </div>
        ) : (
          <>
            <InstrumentCard track={track} onPick={() => openInstrumentSelector(track.id)} />
            <MidiCard track={track} />
            <TransportCard
              playing={playing}
              position={position}
              bpm={bpm}
              hasInstrument={track.instrumentPath !== null}
              hasClip={track.clips.length > 0}
              onPlay={() => send({ type: playing ? 'transport.stop' : 'transport.play' })}
              onStop={() => send({ type: 'transport.stop' })}
            />
            <EffectsCard track={track} />
          </>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------

function Header({ connected }: { connected: boolean }) {
  return (
    <div className="flex items-center justify-between pb-3 border-b border-daw-border">
      <h1 className="text-lg font-semibold tracking-wide">
        moonlitt <span className="text-[#666] font-normal">player</span>
      </h1>
      <div className="flex items-center gap-2">
        <div className={`w-2 h-2 rounded-full ${connected ? 'bg-green-400' : 'bg-red-400'}`} />
        <span className="text-xs text-[#888]">{connected ? 'engine connected' : 'offline'}</span>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------

function Card({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="bg-daw-panel border border-daw-border rounded-lg overflow-hidden">
      <div className="px-4 py-2 border-b border-daw-border bg-daw-control/30">
        <span className="text-[10px] uppercase tracking-widest text-[#888] font-semibold">
          {title}
        </span>
      </div>
      <div className="p-4">{children}</div>
    </section>
  );
}

// ---------------------------------------------------------------------------

function InstrumentCard({ track, onPick }: { track: Track; onPick: () => void }) {
  const loaded = track.instrumentPath !== null;
  const display = track.instrumentName ?? track.instrumentPath?.split('/').pop() ?? '—';
  return (
    <Card title="1 · Instrument (sound source)">
      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={onPick}
          className="px-4 py-2 rounded bg-daw-accent hover:bg-daw-accent/80 text-white text-sm font-medium transition-colors"
        >
          {loaded ? 'Change…' : 'Pick instrument…'}
        </button>
        <div className="flex-1 min-w-0">
          {loaded ? (
            <>
              <div className="text-sm text-[#e0e0e0] truncate" title={track.instrumentPath ?? ''}>
                {display}
              </div>
              <div className="text-[10px] text-[#666] truncate">{track.instrumentPath}</div>
            </>
          ) : (
            <div className="text-xs text-[#666]">SF2 / VST3 / CLAP — none loaded</div>
          )}
        </div>
      </div>
    </Card>
  );
}

// ---------------------------------------------------------------------------

function MidiCard({ track }: { track: Track }) {
  const [dragOver, setDragOver] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const upload = useCallback(
    async (file: File) => {
      if (!file.name.match(/\.midi?$/i)) {
        setError(`Not a MIDI file: ${file.name}`);
        return;
      }
      setError(null);
      setBusy(true);
      const ok = await uploadMidiFile(file, track.id);
      setBusy(false);
      if (!ok) setError('Upload failed (see server logs)');
    },
    [track.id],
  );

  const onDrop = useCallback(
    async (e: React.DragEvent) => {
      e.preventDefault();
      setDragOver(false);
      const file = e.dataTransfer.files[0];
      if (file) await upload(file);
    },
    [upload],
  );

  const clip = track.clips[0] ?? null;

  return (
    <Card title="2 · MIDI file (notes)">
      <div
        onDragOver={(e) => {
          e.preventDefault();
          setDragOver(true);
        }}
        onDragLeave={() => setDragOver(false)}
        onDrop={onDrop}
        onClick={() => fileInputRef.current?.click()}
        className={`cursor-pointer rounded border-2 border-dashed p-6 text-center transition-colors ${
          dragOver
            ? 'border-daw-accent bg-daw-accent/10'
            : 'border-daw-border hover:border-daw-accent/60 hover:bg-daw-control/30'
        }`}
      >
        {clip !== null ? (
          <>
            <div className="text-sm text-[#e0e0e0]">{clip.name}</div>
            <div className="text-[11px] text-[#888] mt-1">
              {clip.lengthBars.toFixed(1)} bars · click or drop to replace
            </div>
          </>
        ) : (
          <>
            <div className="text-sm text-[#aaa]">
              {busy ? 'Uploading…' : 'Drop a .mid file here, or click to choose'}
            </div>
            <div className="text-[11px] text-[#666] mt-1">.mid / .midi</div>
          </>
        )}
        <input
          ref={fileInputRef}
          type="file"
          accept=".mid,.midi"
          onChange={(e) => {
            const file = e.target.files?.[0];
            if (file) upload(file);
            e.target.value = '';
          }}
          className="hidden"
        />
      </div>
      {error !== null && (
        <div className="mt-2 text-[11px] text-red-400">{error}</div>
      )}
    </Card>
  );
}

// ---------------------------------------------------------------------------

function formatBarsBeats(ticks: number): string {
  const tpq = 480;
  const beatsPerBar = 4;
  const ticksPerBar = tpq * beatsPerBar;
  const bar = Math.floor(ticks / ticksPerBar) + 1;
  const beat = Math.floor((ticks % ticksPerBar) / tpq) + 1;
  return `${bar}.${beat}`;
}

function TransportCard({
  playing,
  position,
  bpm,
  hasInstrument,
  hasClip,
  onPlay,
  onStop,
}: {
  playing: boolean;
  position: number;
  bpm: number;
  hasInstrument: boolean;
  hasClip: boolean;
  onPlay: () => void;
  onStop: () => void;
}) {
  const ready = hasInstrument && hasClip;
  const missing = [
    !hasInstrument && 'instrument',
    !hasClip && 'MIDI file',
  ].filter(Boolean).join(' + ');

  return (
    <Card title="3 · Transport">
      <div className="flex items-center gap-4">
        <button
          type="button"
          onClick={onPlay}
          disabled={!ready}
          className={`px-5 py-2.5 rounded text-sm font-semibold transition-colors disabled:opacity-30 disabled:cursor-not-allowed ${
            playing
              ? 'bg-daw-accent text-white'
              : 'bg-daw-control hover:bg-daw-border text-[#e0e0e0]'
          }`}
          title={ready ? 'Space to toggle' : `Need: ${missing}`}
        >
          {playing ? '❚❚ Pause' : '▶ Play'}
        </button>
        <button
          type="button"
          onClick={onStop}
          disabled={!ready}
          className="px-3 py-2.5 rounded bg-daw-control hover:bg-daw-border text-[#e0e0e0] text-sm transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
        >
          ■ Stop
        </button>
        <div className="ml-auto flex items-center gap-4 font-mono text-sm">
          <span>
            <span className="text-[10px] text-[#888] mr-1">POS</span>
            {formatBarsBeats(position)}
          </span>
          <span>
            <span className="text-[10px] text-[#888] mr-1">BPM</span>
            {bpm.toFixed(1)}
          </span>
        </div>
      </div>
      {!ready && (
        <div className="mt-3 text-[11px] text-[#888]">
          Need {missing} above before playback.
        </div>
      )}
    </Card>
  );
}

// ---------------------------------------------------------------------------

function EffectsCard({ track }: { track: Track }) {
  const send = useSessionStore((s) => s.send);
  const [adding, setAdding] = useState(false);

  const handleAdd = useCallback(
    (effectType: string) => {
      send({ type: 'insert.add', trackId: track.id, effectType });
      setAdding(false);
    },
    [send, track.id],
  );

  return (
    <Card title="4 · Effects (insert chain)">
      {track.inserts.length === 0 ? (
        <div className="text-xs text-[#666] mb-3">No effects yet.</div>
      ) : (
        <div className="flex flex-col gap-2 mb-3">
          {track.inserts.map((insert) => (
            <InsertRow key={insert.id} trackId={track.id} insert={insert} />
          ))}
        </div>
      )}

      {adding ? (
        <select
          autoFocus
          defaultValue=""
          onChange={(e) => {
            if (e.target.value) handleAdd(e.target.value);
          }}
          onBlur={() => setAdding(false)}
          className="w-full bg-daw-control border border-daw-accent rounded px-3 py-2 text-sm text-[#e0e0e0] outline-none"
        >
          <option value="" disabled>Select an effect…</option>
          {EFFECT_TYPES.map((fx) => (
            <option key={fx.value} value={fx.value}>{fx.label}</option>
          ))}
        </select>
      ) : (
        <button
          type="button"
          onClick={() => setAdding(true)}
          className="px-3 py-1.5 rounded bg-daw-control hover:bg-daw-border text-[#ccc] text-xs font-medium transition-colors"
        >
          + Add effect…
        </button>
      )}
    </Card>
  );
}

function InsertRow({ trackId, insert }: { trackId: number; insert: Insert }) {
  const send = useSessionStore((s) => s.send);
  const setInsertParam = useMixerStore((s) => s.setInsertParam);
  const removeInsertLocal = useMixerStore((s) => s.removeInsert);

  const handleParam = useCallback(
    (param: ParamMeta, value: number) => {
      setInsertParam(trackId, insert.id, param.id, value);
      send({
        type: 'insert.set_param',
        trackId,
        insertId: insert.id,
        paramId: param.id,
        value,
      });
    },
    [trackId, insert.id, setInsertParam, send],
  );

  const handleRemove = useCallback(() => {
    removeInsertLocal(trackId, insert.id);
    send({ type: 'insert.remove', trackId, insertId: insert.id });
  }, [trackId, insert.id, removeInsertLocal, send]);

  return (
    <div className="rounded border border-daw-border bg-daw-control/40">
      <div className="flex items-center gap-2 px-3 py-2 border-b border-daw-border">
        <span className="w-2 h-2 rounded-full bg-green-400 shrink-0" />
        <span className="text-sm text-[#e0e0e0] flex-1 truncate">{insert.name}</span>
        <button
          type="button"
          onClick={handleRemove}
          className="text-[#666] hover:text-red-400 transition-colors px-1"
          title="Remove"
        >
          ×
        </button>
      </div>
      {insert.params.length > 0 && (
        <div className="p-3 flex flex-col gap-2">
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
    <div className="grid grid-cols-[8rem_1fr_4rem] items-center gap-3">
      <label className="text-[11px] text-[#bbb] truncate" title={param.name}>
        {param.name}
      </label>
      <input
        type="range"
        min={param.min}
        max={param.max}
        step={step}
        value={param.value}
        onChange={(e) => onChange(parseFloat(e.target.value))}
        className="w-full accent-daw-accent"
      />
      <span className="text-[11px] text-[#888] text-right tabular-nums">
        {formatValue(param.value, param)}
      </span>
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
