import { useCallback } from 'react';
import type { Track } from '../stores/mixer';
import { useMixerStore } from '../stores/mixer';
import { useSessionStore } from '../stores/session';

interface TrackHeaderProps {
  track: Track;
  selected: boolean;
  onSelect: () => void;
}

export function TrackHeader({ track, selected, onSelect }: TrackHeaderProps) {
  const send = useSessionStore((s) => s.send);
  const setTrackMute = useMixerStore((s) => s.setTrackMute);
  const setTrackSolo = useMixerStore((s) => s.setTrackSolo);

  const handleMute = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      const muted = !track.muted;
      setTrackMute(track.id, muted);
      send({ type: 'track.set_mute', trackId: track.id, muted });
    },
    [track.id, track.muted, send, setTrackMute],
  );

  const handleSolo = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      const solo = !track.solo;
      setTrackSolo(track.id, solo);
      send({ type: 'track.set_solo', trackId: track.id, solo });
    },
    [track.id, track.solo, send, setTrackSolo],
  );

  return (
    <div
      className={`w-[120px] shrink-0 flex items-center gap-1.5 px-1 cursor-pointer border-r border-daw-border
        ${selected ? 'bg-daw-control' : 'bg-daw-panel hover:bg-daw-control/50'}
        transition-colors`}
      onClick={onSelect}
    >
      {/* Color bar */}
      <div
        className="w-1 self-stretch rounded-sm shrink-0"
        style={{ backgroundColor: track.color }}
      />

      {/* Name + buttons */}
      <div className="flex-1 flex flex-col gap-0.5 min-w-0 py-1">
        <span className="text-[11px] text-[#ccc] truncate" title={track.name}>
          {track.name}
        </span>
        <div className="flex gap-1">
          <button
            onClick={handleMute}
            className={`w-4 h-4 text-[9px] font-bold rounded
              ${track.muted ? 'bg-red-500/80 text-white' : 'bg-daw-bg text-[#666] hover:text-white'}`}
          >
            M
          </button>
          <button
            onClick={handleSolo}
            className={`w-4 h-4 text-[9px] font-bold rounded
              ${track.solo ? 'bg-yellow-500/80 text-black' : 'bg-daw-bg text-[#666] hover:text-white'}`}
          >
            S
          </button>
        </div>
      </div>
    </div>
  );
}
