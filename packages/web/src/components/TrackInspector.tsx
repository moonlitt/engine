import { useState, useCallback } from 'react';
import { useMixerStore } from '../stores/mixer';
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
          <div className="flex flex-col gap-1 mb-2">
            {track.inserts.map((insert) => (
              <div
                key={insert.id}
                className="flex items-center gap-2 px-2 py-1 rounded bg-daw-control text-xs"
              >
                <div
                  className={`w-2 h-2 rounded-full shrink-0 ${
                    insert.bypassed ? 'bg-[#555]' : 'bg-green-400'
                  }`}
                  title={insert.bypassed ? 'Bypassed' : 'Active'}
                />
                <span className="text-[#ccc] truncate">{insert.name}</span>
              </div>
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
