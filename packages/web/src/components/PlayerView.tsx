import { useEffect, useRef } from 'react';
import { useMixerStore } from '../stores/mixer';
import { useTransportStore } from '../stores/transport';
import { useSessionStore } from '../stores/session';
import { Header } from './Header';
import { TrackCard } from './TrackCard';

/**
 * Multi-track player. Vertical list of track cards, each with its own
 * source / MIDI / mute-solo / volume / effect chain. Header pinned at top.
 *
 * Auto-creates the first track on connect so the user has something to act
 * on immediately rather than landing on an empty screen.
 */
export function PlayerView() {
  const tracks = useMixerStore((s) => s.tracks);
  const send = useSessionStore((s) => s.send);
  const connected = useSessionStore((s) => s.connected);
  const playing = useTransportStore((s) => s.playing);
  const position = useTransportStore((s) => s.position);
  const bpm = useTransportStore((s) => s.bpm);

  const autoCreatedRef = useRef(false);
  useEffect(() => {
    if (!connected || autoCreatedRef.current) return;
    autoCreatedRef.current = true;
    if (tracks.length === 0) {
      send({ type: 'track.add' });
    }
  }, [connected, tracks.length, send]);

  return (
    <div className="h-screen overflow-y-auto bg-daw-bg text-[#e0e0e0] font-sans">
      <Header
        connected={connected}
        playing={playing}
        position={position}
        bpm={bpm}
        onPlay={() => send({ type: playing ? 'transport.stop' : 'transport.play' })}
        onStop={() => send({ type: 'transport.stop' })}
      />
      <div className="max-w-[820px] mx-auto px-6 py-6 flex flex-col gap-4">
        {tracks.length === 0 ? (
          <div className="text-center text-[#666] py-12">
            {connected ? 'Initializing first track…' : 'Connecting to engine…'}
          </div>
        ) : (
          tracks.map((track) => <TrackCard key={track.id} track={track} />)
        )}
        {tracks.length > 0 && (
          <button
            type="button"
            onClick={() => send({ type: 'track.add' })}
            className="self-center mt-2 px-4 py-2 rounded bg-daw-control hover:bg-daw-border text-[#ccc] text-sm transition-colors"
          >
            + Add Track
          </button>
        )}
      </div>
    </div>
  );
}
