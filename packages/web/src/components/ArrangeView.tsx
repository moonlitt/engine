import { useCallback, useRef, useState } from 'react';
import { useMixerStore } from '../stores/mixer';
import { useTransportStore } from '../stores/transport';
import { useSessionStore } from '../stores/session';
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
    <div className="flex flex-col h-full overflow-hidden">
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
                  <div className="absolute inset-0 border-2 border-dashed border-daw-accent/50 rounded pointer-events-none z-10 flex items-center justify-center">
                    <span className="text-[10px] text-daw-accent/70 bg-daw-bg/80 px-2 py-0.5 rounded">
                      Drop .mid file
                    </span>
                  </div>
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

      {/* Empty state */}
      {tracks.length === 0 && (
        <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
          <span className="text-[#444] text-xs">
            No tracks yet -- click "+ Add Track" to begin
          </span>
        </div>
      )}
    </div>
  );
}
