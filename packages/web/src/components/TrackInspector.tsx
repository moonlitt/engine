import { useState, useCallback } from 'react';
import type { ParamMeta } from '@moonlitt/protocol';
import { useMixerStore, type Insert } from '../stores/mixer';
import { useSessionStore } from '../stores/session';
import { InstrumentSelector } from './InstrumentSelector';

const EFFECT_TYPES: readonly { label: string; value: string }[] = [
  { label: 'EQ', value: 'eq' },
  { label: 'Compressor', value: 'compressor' },
  { label: 'Limiter', value: 'limiter' },
  { label: 'Gate', value: 'gate' },
  { label: 'De-esser', value: 'de-esser' },
  { label: 'Reverb', value: 'reverb' },
  { label: 'Dattorro Reverb', value: 'dattorro-reverb' },
  { label: 'Delay', value: 'delay' },
  { label: 'Chorus', value: 'chorus' },
  { label: 'Flanger', value: 'flanger' },
  { label: 'Phaser', value: 'phaser' },
  { label: 'Tremolo', value: 'tremolo' },
  { label: 'Saturator', value: 'saturator' },
  { label: 'Bitcrusher', value: 'bitcrusher' },
  { label: 'Multiband Compressor', value: 'multiband-compressor' },
  { label: 'Auto Filter', value: 'auto-filter' },
  { label: 'Pitch Shifter', value: 'pitch-shifter' },
  { label: 'Gain', value: 'gain' },
  { label: 'Stereo Width', value: 'stereo-width' },
];

export function TrackInspector() {
  const selectedTrackId = useMixerStore((s) => s.selectedTrackId);
  const tracks = useMixerStore((s) => s.tracks);
  const send = useSessionStore((s) => s.send);

  const [instrumentSelectorOpen, setInstrumentSelectorOpen] = useState(false);

  const track = tracks.find((t) => t.id === selectedTrackId) ?? null;

  const handleAddInsert = useCallback(
    (effectType: string) => {
      if (track === null) return;
      send({ type: 'insert.add', trackId: track.id, effectType });
    },
    [track, send],
  );

  const handleLoadInstrument = useCallback(
    (path: string) => {
      if (track === null) return;
      send({ type: 'track.load_instrument', trackId: track.id, path });
      setInstrumentSelectorOpen(false);
    },
    [track, send],
  );

  if (track === null) {
    return (
      <div className="flex items-center justify-center h-full text-[#555] text-xs">
        Select a track
      </div>
    );
  }

  const instrumentDisplay = track.instrumentPath
    ? track.instrumentName ?? track.instrumentPath.split('/').pop() ?? track.instrumentPath
    : 'No instrument loaded';

  return (
    <div className="flex flex-col gap-4 h-full overflow-y-auto">
      {/* Header */}
      <div className="flex items-center gap-2">
        <div
          className="w-3 h-3 rounded-full shrink-0"
          style={{ backgroundColor: track.color }}
        />
        <span className="text-sm font-medium text-[#e0e0e0] truncate">
          {track.name}
        </span>
      </div>

      {/* Instrument */}
      <div>
        <div className="text-[10px] text-[#888] uppercase tracking-wider mb-1.5">
          Instrument
        </div>
        <div className="text-xs text-[#aaa] truncate mb-2" title={track.instrumentPath ?? undefined}>
          {instrumentDisplay}
        </div>
        <button
          type="button"
          onClick={() => setInstrumentSelectorOpen(true)}
          className="text-[10px] px-2 py-1 rounded bg-daw-control hover:bg-daw-border text-[#ccc] transition-colors"
        >
          Load
        </button>
      </div>

      {/* Insert Chain */}
      <div className="flex-1">
        <div className="text-[10px] text-[#888] uppercase tracking-wider mb-1.5">
          Inserts
        </div>

        {track.inserts.length === 0 ? (
          <div className="text-xs text-[#555] mb-2">No inserts</div>
        ) : (
          <div className="flex flex-col gap-2 mb-2">
            {track.inserts.map((insert) => (
              <InsertCard key={insert.id} trackId={track.id} insert={insert} />
            ))}
          </div>
        )}

        <InsertAdder onAdd={handleAddInsert} />
      </div>

      {/* Instrument Selector Modal */}
      <InstrumentSelector
        open={instrumentSelectorOpen}
        onLoad={handleLoadInstrument}
        onClose={() => setInstrumentSelectorOpen(false)}
      />
    </div>
  );
}

function InsertCard({ trackId, insert }: { trackId: number; insert: Insert }) {
  const send = useSessionStore((s) => s.send);
  const setInsertParam = useMixerStore((s) => s.setInsertParam);
  const removeInsertLocal = useMixerStore((s) => s.removeInsert);
  const [expanded, setExpanded] = useState(true);

  const handleParamChange = useCallback(
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
    <div className="rounded bg-daw-control border border-daw-border">
      <div className="flex items-center gap-2 px-2 py-1.5 text-xs">
        <button
          type="button"
          onClick={() => setExpanded((e) => !e)}
          className={`w-3 text-center text-[#888] hover:text-[#ccc] transition-colors ${
            expanded ? 'rotate-90' : ''
          }`}
          aria-label={expanded ? 'Collapse' : 'Expand'}
        >
          ▶
        </button>
        <div
          className={`w-2 h-2 rounded-full shrink-0 ${
            insert.bypassed ? 'bg-[#555]' : 'bg-green-400'
          }`}
          title={insert.bypassed ? 'Bypassed' : 'Active'}
        />
        <span className="text-[#ccc] truncate flex-1">{insert.name}</span>
        <button
          type="button"
          onClick={handleRemove}
          className="text-[#666] hover:text-red-400 transition-colors px-1"
          title="Remove insert"
        >
          ×
        </button>
      </div>

      {expanded && insert.params.length > 0 && (
        <div className="px-2 pb-2 pt-1 flex flex-col gap-1.5 border-t border-daw-border">
          {insert.params.map((p) => (
            <ParamSlider
              key={p.id}
              param={p}
              onChange={(value) => handleParamChange(p, value)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function ParamSlider({
  param,
  onChange,
}: {
  param: ParamMeta;
  onChange: (value: number) => void;
}) {
  const range = param.max - param.min;
  const stepped = param.stepCount > 0;
  const step = stepped ? range / param.stepCount : range / 1000;
  return (
    <div className="grid grid-cols-[6.5rem_1fr_3.5rem] items-center gap-2">
      <label className="text-[10px] text-[#aaa] truncate" title={param.name}>
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
      <span className="text-[10px] text-[#888] text-right tabular-nums">
        {formatValue(param.value, param)}
      </span>
    </div>
  );
}

function formatValue(value: number, param: ParamMeta): string {
  // Coarse heuristic: treat 0..1 ranges as percentages.
  if (param.min === 0 && param.max === 1) {
    return `${Math.round(value * 100)}%`;
  }
  if (param.stepCount > 0 && param.stepCount <= 1) {
    return value >= 0.5 ? 'on' : 'off';
  }
  const abs = Math.abs(value);
  if (abs >= 1000) return value.toFixed(0);
  if (abs >= 100) return value.toFixed(1);
  if (abs >= 10) return value.toFixed(2);
  return value.toFixed(3);
}

function InsertAdder({ onAdd }: { onAdd: (effectType: string) => void }) {
  const [selecting, setSelecting] = useState(false);

  const handleSelect = useCallback(
    (e: React.ChangeEvent<HTMLSelectElement>) => {
      const value = e.target.value;
      if (value === '') return;
      onAdd(value);
      setSelecting(false);
    },
    [onAdd],
  );

  if (!selecting) {
    return (
      <button
        type="button"
        onClick={() => setSelecting(true)}
        className="text-[10px] px-2 py-1 rounded bg-daw-control hover:bg-daw-border text-[#ccc] transition-colors"
      >
        + Add Insert
      </button>
    );
  }

  return (
    <select
      onChange={handleSelect}
      onBlur={() => setSelecting(false)}
      className="w-full text-xs bg-daw-control border border-daw-border rounded px-2 py-1 text-[#ccc] outline-none focus:border-daw-accent"
      autoFocus
      defaultValue=""
    >
      <option value="" disabled>
        Select effect...
      </option>
      {EFFECT_TYPES.map((fx) => (
        <option key={fx.value} value={fx.value}>
          {fx.label}
        </option>
      ))}
    </select>
  );
}
