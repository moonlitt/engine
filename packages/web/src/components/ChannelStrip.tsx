import { useCallback } from 'react';
import type { Track } from '../stores/mixer';
import { useMixerStore } from '../stores/mixer';
import { useSessionStore } from '../stores/session';
import { Meter } from './Meter';
import { Fader } from './Fader';
import { PanKnob } from './PanKnob';

interface ChannelStripProps {
  track: Track;
}

export function ChannelStrip({ track }: ChannelStripProps) {
  const send = useSessionStore((s) => s.send);
  const selectTrack = useMixerStore((s) => s.selectTrack);
  const selectedTrackId = useMixerStore((s) => s.selectedTrackId);
  const setTrackVolume = useMixerStore((s) => s.setTrackVolume);
  const setTrackPan = useMixerStore((s) => s.setTrackPan);
  const setTrackMute = useMixerStore((s) => s.setTrackMute);
  const setTrackSolo = useMixerStore((s) => s.setTrackSolo);

  const isSelected = selectedTrackId === track.id;

  const handleVolume = useCallback(
    (db: number) => {
      setTrackVolume(track.id, db);
      send({ type: 'track.set_volume', trackId: track.id, db });
    },
    [track.id, send, setTrackVolume],
  );

  const handlePan = useCallback(
    (pan: number) => {
      setTrackPan(track.id, pan);
      send({ type: 'track.set_pan', trackId: track.id, pan });
    },
    [track.id, send, setTrackPan],
  );

  const handleMute = useCallback(() => {
    const muted = !track.muted;
    setTrackMute(track.id, muted);
    send({ type: 'track.set_mute', trackId: track.id, muted });
  }, [track.id, track.muted, send, setTrackMute]);

  const handleSolo = useCallback(() => {
    const solo = !track.solo;
    setTrackSolo(track.id, solo);
    send({ type: 'track.set_solo', trackId: track.id, solo });
  }, [track.id, track.solo, send, setTrackSolo]);

  return (
    <div
      className={`flex flex-col items-center gap-1 px-1.5 py-2 rounded min-w-[52px]
        ${isSelected ? 'bg-daw-control' : 'bg-daw-panel hover:bg-daw-control/50'}
        cursor-pointer transition-colors`}
      onClick={() => selectTrack(track.id)}
    >
      {/* Mute / Solo */}
      <div className="flex gap-1">
        <button
          onClick={(e) => { e.stopPropagation(); handleMute(); }}
          className={`w-5 h-5 text-[10px] font-bold rounded
            ${track.muted ? 'bg-red-500/80 text-white' : 'bg-daw-bg text-[#888] hover:text-white'}`}
        >
          M
        </button>
        <button
          onClick={(e) => { e.stopPropagation(); handleSolo(); }}
          className={`w-5 h-5 text-[10px] font-bold rounded
            ${track.solo ? 'bg-yellow-500/80 text-black' : 'bg-daw-bg text-[#888] hover:text-white'}`}
        >
          S
        </button>
      </div>

      {/* Pan */}
      <PanKnob value={track.pan} onChange={handlePan} />

      {/* Meter + Fader side by side */}
      <div className="flex gap-0.5 items-end">
        <Meter trackId={track.id} width={14} height={80} />
        <Fader value={track.volume} onChange={handleVolume} height={80} />
      </div>

      {/* Track name + color indicator */}
      <div className="flex items-center gap-1 mt-0.5 max-w-full">
        <div
          className="w-2 h-2 rounded-full shrink-0"
          style={{ backgroundColor: track.color }}
        />
        <span className="text-[10px] text-[#aaa] truncate max-w-[40px]" title={track.name}>
          {track.name}
        </span>
      </div>
    </div>
  );
}

// -- Master strip (separate, slightly different layout) --

interface MasterStripProps {
  volume: number;
}

export function MasterStrip({ volume }: MasterStripProps) {
  const send = useSessionStore((s) => s.send);
  const setMasterVolume = useMixerStore((s) => s.setMasterVolume);

  const handleVolume = useCallback(
    (db: number) => {
      setMasterVolume(db);
      send({ type: 'master.set_volume', db });
    },
    [send, setMasterVolume],
  );

  return (
    <div className="flex flex-col items-center gap-1 px-2 py-2 bg-daw-control rounded min-w-[60px]">
      <span className="text-[10px] text-daw-accent font-bold">MASTER</span>

      {/* Meter + Fader */}
      <div className="flex gap-0.5 items-end">
        <Meter trackId="master" width={18} height={80} />
        <Fader value={volume} onChange={handleVolume} height={80} />
      </div>
    </div>
  );
}
