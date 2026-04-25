import { useMixerStore } from '../stores/mixer';
import { useTransportStore } from '../stores/transport';
import { useSessionStore } from '../stores/session';
import { Header } from './Header';
import { TrackCard } from './TrackCard';
import { MidiBar } from './MidiBar';

/**
 * Player layout — top-down: header → MIDI bar → track list.
 *
 * The MIDI upload area is global. Tracks are created server-side from the
 * MIDI's channels, one per channel — so the UI never asks the user to
 * decide "which track does this MIDI go on". Each track row gets its own
 * instrument picker, mute/solo, volume, and effects.
 */
export function PlayerView() {
  const tracks = useMixerStore((s) => s.tracks);
  const send = useSessionStore((s) => s.send);
  const connected = useSessionStore((s) => s.connected);
  const playing = useTransportStore((s) => s.playing);
  const position = useTransportStore((s) => s.position);
  const bpm = useTransportStore((s) => s.bpm);

  // First clip across any track is the "currently loaded MIDI" for display
  // purposes. The server keeps a single global sequencer, so by construction
  // every clip in the project comes from the same upload.
  const loadedMidi: string | null = (() => {
    for (const t of tracks) {
      const c = t.clips[0];
      if (c) return c.name.replace(/\s*\(ch \d+\)$/, ''); // strip "(ch 1)" suffix
    }
    return null;
  })();

  const hasTracks = tracks.length > 0;

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

      <MidiBar loadedName={loadedMidi} trackCount={tracks.length} hero={!hasTracks} />

      {hasTracks && (
        <div className="max-w-[820px] mx-auto px-6 py-4 flex flex-col gap-4">
          {tracks.map((track) => (
            <TrackCard key={track.id} track={track} />
          ))}
        </div>
      )}

      {!connected && (
        <div className="text-center text-[#666] text-xs mt-6">Connecting to engine…</div>
      )}
    </div>
  );
}
