import { useState } from 'react';
import { useMixerStore } from '../stores/mixer';
import { ChannelStrip, MasterStrip } from './ChannelStrip';

export function Mixer() {
  const tracks = useMixerStore((s) => s.tracks);
  const masterVolume = useMixerStore((s) => s.masterVolume);
  const [collapsed, setCollapsed] = useState(false);

  if (collapsed) {
    return (
      <div className="h-8 bg-daw-panel border-t-2 border-daw-border flex items-center px-3">
        <button
          onClick={() => setCollapsed(false)}
          className="text-[10px] text-[#888] hover:text-white transition-colors"
        >
          Mixer [+]
        </button>
      </div>
    );
  }

  return (
    <div className="bg-daw-panel border-t-2 border-daw-border">
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-1 border-b border-daw-border">
        <span className="text-[10px] text-[#888] uppercase tracking-wider">Mixer</span>
        <button
          onClick={() => setCollapsed(true)}
          className="text-[10px] text-[#888] hover:text-white transition-colors"
        >
          [-]
        </button>
      </div>

      {/* Strips */}
      <div className="flex items-stretch gap-0.5 p-2 overflow-x-auto">
        {/* Track strips */}
        {tracks.map((track) => (
          <ChannelStrip key={track.id} track={track} />
        ))}

        {/* Empty state */}
        {tracks.length === 0 && (
          <div className="flex items-center justify-center text-[#555] text-xs px-4 py-8">
            No tracks
          </div>
        )}

        {/* Divider */}
        {tracks.length > 0 && (
          <div className="w-px bg-daw-border mx-1 self-stretch" />
        )}

        {/* Master */}
        <MasterStrip volume={masterVolume} />
      </div>
    </div>
  );
}
