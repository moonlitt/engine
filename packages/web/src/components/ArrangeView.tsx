import { useCallback, useRef, useState } from 'react';
import { useMixerStore } from '../stores/mixer';
import { useTransportStore } from '../stores/transport';
import { useSessionStore } from '../stores/session';
import { useUiStore } from '../stores/ui';
import { uploadMidiFile } from '../services/upload';
import { TimelineRuler } from './TimelineRuler';
import { TrackHeader } from './TrackHeader';
import { MidiClip } from './MidiClip';
import { Playhead } from './Playhead';

const PIXELS_PER_BAR = 100;
const TRACK_HEIGHT = 56;
const TOTAL_BARS = 64;

export function ArrangeView() {
  const tracks = useMixerStore((s) => s.tracks);
  const selectedTrackId = useMixerStore((s) => s.selectedTrackId);
  const selectTrack = useMixerStore((s) => s.selectTrack);
  const position = useTransportStore((s) => s.position);
  const send = useSessionStore((s) => s.send);
  const openInstrumentSelector = useUiStore((s) => s.openInstrumentSelector);

  const [scrollLeft, setScrollLeft] = useState(0);
  const [dragOverTrackId, setDragOverTrackId] = useState<number | null>(null);
  const lanesRef = useRef<HTMLDivElement>(null);

  const handleScroll = useCallback(() => {
    if (lanesRef.current) {
      setScrollLeft(lanesRef.current.scrollLeft);
    }
  }, []);

  const handleAddTrack = useCallback(() => {
    send({ type: 'track.add' });
  }, [send]);

  const handleDragOver = useCallback((e: React.DragEvent, trackId: number) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = 'copy';
    setDragOverTrackId(trackId);
  }, []);

  const handleDragLeave = useCallback(() => {
    setDragOverTrackId(null);
  }, []);

  const handleDrop = useCallback(async (e: React.DragEvent, trackId: number) => {
    e.preventDefault();
    setDragOverTrackId(null);

    const file = e.dataTransfer.files[0];
    if (!file) return;
    if (!file.name.endsWith('.mid') && !file.name.endsWith('.midi')) return;

    await uploadMidiFile(file, trackId);
  }, []);

  const handleFileUpload = useCallback(async (file: File, trackId: number) => {
    await uploadMidiFile(file, trackId);
  }, []);

  // Convert transport position (ticks) to pixels.
  // Position is in MIDI ticks (480 ticks per beat, 4 beats per bar).
  const ticksPerBar = 480 * 4;
  const positionPx = (position / ticksPerBar) * PIXELS_PER_BAR;

  const totalWidth = TOTAL_BARS * PIXELS_PER_BAR;

  return (
    <div className="relative flex flex-col h-full overflow-hidden">
      {/* Timeline ruler */}
      <TimelineRuler
        pixelsPerBar={PIXELS_PER_BAR}
        totalBars={TOTAL_BARS}
        scrollLeft={scrollLeft}
      />

      {/* Main area: headers (fixed) | lanes (scrollable) */}
      <div className="flex-1 flex overflow-hidden relative">
        {/* Playhead overlay spanning all lanes */}
        <Playhead positionPx={positionPx} scrollLeft={scrollLeft} />

        {/* Left column: track headers (fixed, scrolls vertically with lanes) */}
        <div className="w-[120px] shrink-0 bg-daw-panel border-r border-daw-border overflow-hidden">
          <div className="overflow-y-auto h-full" style={{ scrollbarWidth: 'none' }}>
            {tracks.map((track) => (
              <div
                key={track.id}
                className="border-b border-daw-border/50"
                style={{ height: `${TRACK_HEIGHT}px` }}
              >
                <TrackHeader
                  track={track}
                  selected={selectedTrackId === track.id}
                  onSelect={() => selectTrack(track.id)}
                  onFileUpload={handleFileUpload}
                />
              </div>
            ))}

            {/* Add Track button */}
            <div
              className="flex items-center justify-center"
              style={{ height: `${TRACK_HEIGHT}px` }}
            >
              <button
                onClick={handleAddTrack}
                className="text-[11px] text-[#555] hover:text-daw-accent transition-colors"
              >
                + Add Track
              </button>
            </div>
          </div>
        </div>

        {/* Right column: track lanes (scrolls horizontally) */}
        <div
          ref={lanesRef}
          className="flex-1 overflow-x-auto overflow-y-auto"
          onScroll={handleScroll}
        >
          <div style={{ width: `${totalWidth}px`, minHeight: '100%' }}>
            {tracks.map((track) => (
              <div
                key={track.id}
                className={`relative border-b border-daw-border/50 transition-colors ${
                  dragOverTrackId === track.id ? 'bg-daw-accent/10' : ''
                }`}
                style={{ height: `${TRACK_HEIGHT}px` }}
                onDragOver={(e) => handleDragOver(e, track.id)}
                onDragLeave={handleDragLeave}
                onDrop={(e) => handleDrop(e, track.id)}
              >
                {/* Bar grid lines */}
                {Array.from({ length: TOTAL_BARS }, (_, i) => (
                  <div
                    key={i}
                    className="absolute top-0 bottom-0 border-l border-daw-border/30"
                    style={{ left: `${i * PIXELS_PER_BAR}px` }}
                  />
                ))}

                {/* Drop indicator */}
                {dragOverTrackId === track.id && (
                  <div className="absolute inset-0 border-2 border-dashed border-daw-accent rounded pointer-events-none z-10 flex items-center justify-center bg-daw-accent/10">
                    <span className="text-xs text-daw-accent bg-daw-bg/80 px-3 py-1 rounded font-medium">
                      Drop .mid file
                    </span>
                  </div>
                )}

                {/* Inline lane CTAs — shown when track is incomplete, anchored
                    at the visible left edge regardless of horizontal scroll. */}
                {dragOverTrackId !== track.id && (
                  <EmptyLaneCTA
                    track={track}
                    scrollLeft={scrollLeft}
                    onPickInstrument={() => openInstrumentSelector(track.id)}
                    onUploadMidi={(file) => handleFileUpload(file, track.id)}
                  />
                )}

                {/* Clips */}
                {track.clips.map((clip) => (
                  <MidiClip
                    key={clip.id}
                    clip={clip}
                    color={track.color}
                    pixelsPerBar={PIXELS_PER_BAR}
                  />
                ))}
              </div>
            ))}

            {/* Empty filler row matching add-track height */}
            <div style={{ height: `${TRACK_HEIGHT}px` }} />
          </div>
        </div>
      </div>

      {/* Empty workspace — single primary CTA so the user has one
          unmistakable next step. Auto-creates and selects a track. */}
      {tracks.length === 0 && (
        <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
          <div className="text-center pointer-events-auto">
            <button
              onClick={handleAddTrack}
              className="px-6 py-3 bg-daw-accent hover:bg-daw-accent/80 text-white text-sm font-medium rounded-lg shadow-lg transition-colors"
            >
              + Create your first track
            </button>
            <div className="text-[11px] text-[#666] mt-3">
              Then pick an instrument and drop a <span className="text-[#aaa]">.mid</span> file
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

interface EmptyLaneCTAProps {
  track: { id: number; instrumentPath: string | null; clips: { id: number }[] };
  scrollLeft: number;
  onPickInstrument: () => void;
  onUploadMidi: (file: File) => void;
}

/**
 * Inline call-to-action chips inside an incomplete track lane.
 *
 * Why inline: the user is already looking at the lane, so put the actions
 * here rather than expecting them to discover the 220px right inspector or
 * the 120px left header. Order doesn't matter — instrument and MIDI are
 * independent; a track plays sound only once both exist, but either can
 * be done first.
 */
function EmptyLaneCTA({ track, scrollLeft, onPickInstrument, onUploadMidi }: EmptyLaneCTAProps) {
  const fileInputRef = useRef<HTMLInputElement>(null);

  const hasInstrument = track.instrumentPath !== null;
  const hasClip = track.clips.length > 0;
  if (hasInstrument && hasClip) return null;

  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file) onUploadMidi(file);
    e.target.value = '';
  };

  return (
    <div
      className="absolute top-1/2 -translate-y-1/2 z-20 flex items-center gap-2 pointer-events-auto"
      style={{ left: `${scrollLeft + 12}px` }}
    >
      {!hasInstrument && (
        <button
          type="button"
          onClick={onPickInstrument}
          className="px-2.5 py-1 text-[11px] rounded bg-daw-accent/20 hover:bg-daw-accent text-daw-accent hover:text-white border border-daw-accent/40 transition-colors font-medium"
          title="Pick an SF2 / VST3 / CLAP instrument"
        >
          🎹 Pick instrument
        </button>
      )}
      {!hasClip && (
        <button
          type="button"
          onClick={() => fileInputRef.current?.click()}
          className="px-2.5 py-1 text-[11px] rounded bg-[#2a2a2a] hover:bg-[#3a3a3a] text-[#ccc] border border-[#444] transition-colors font-medium"
          title="Click to choose a .mid file, or drag one onto the lane"
        >
          📁 Upload MIDI
        </button>
      )}
      <input
        ref={fileInputRef}
        type="file"
        accept=".mid,.midi"
        onChange={handleFileChange}
        className="hidden"
      />
    </div>
  );
}
