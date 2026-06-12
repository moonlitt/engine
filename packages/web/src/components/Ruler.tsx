import { useEffect, useRef } from 'react';
import { useProjectStore } from '../stores/project';
import { useSessionStore } from '../stores/session';
import { useMetersStore } from '../stores/meters';
import { clipTime } from '../lib/time';

/**
 * Bar ruler + playhead + progress, full width under the transport.
 * Click or drag anywhere to seek (audio thread releases held notes and
 * jumps). The playhead is positioned imperatively from the 60 Hz meter
 * stream so it glides without re-rendering the React tree.
 */
export function Ruler() {
  const midi = useProjectStore((s) => s.midi);
  const send = useSessionStore((s) => s.send);
  const laneRef = useRef<HTMLDivElement>(null);
  const headRef = useRef<HTMLDivElement>(null);
  const fillRef = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);

  const total = midi?.totalTicks ?? 0;

  // Imperative playhead/progress drawing.
  useEffect(() => {
    if (!midi || total <= 0) return;
    const draw = (ticks: number) => {
      const pct = Math.min(100, (ticks / total) * 100);
      if (headRef.current) headRef.current.style.left = `${pct}%`;
      if (fillRef.current) fillRef.current.style.width = `${pct}%`;
    };
    draw(useMetersStore.getState().playheadTicks);
    return useMetersStore.subscribe((s) => draw(s.playheadTicks));
  }, [midi, total]);

  if (!midi || total <= 0) return null;

  const { ticksPerBar, totalBars } = clipTime(midi);
  // Label every Nth bar so the ruler stays readable on long clips.
  const labelEvery = totalBars > 64 ? 8 : totalBars > 24 ? 4 : totalBars > 12 ? 2 : 1;

  const seekFromPointer = (clientX: number) => {
    const lane = laneRef.current;
    if (!lane) return;
    const rect = lane.getBoundingClientRect();
    const frac = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
    send({ type: 'transport.seek', ticks: frac * total });
  };

  return (
    <div
      ref={laneRef}
      className="ruler relative h-9 shrink-0 border-b border-black/50 overflow-hidden"
      onPointerDown={(e) => {
        dragging.current = true;
        (e.target as Element).setPointerCapture?.(e.pointerId);
        seekFromPointer(e.clientX);
      }}
      onPointerMove={(e) => {
        if (dragging.current) seekFromPointer(e.clientX);
      }}
      onPointerUp={() => {
        dragging.current = false;
      }}
      title="点击或拖拽跳转播放位置"
    >
      {/* Elapsed-region tint */}
      <div
        ref={fillRef}
        className="absolute inset-y-0 left-0 bg-daw-accent/[0.07] pointer-events-none"
        style={{ width: 0 }}
      />
      {/* Bar ticks + numbers */}
      {Array.from({ length: totalBars + 1 }, (_, i) => {
        const pct = ((i * ticksPerBar) / total) * 100;
        if (pct > 100) return null;
        const labelled = i % labelEvery === 0;
        return (
          <div key={i} className="absolute inset-y-0 pointer-events-none" style={{ left: `${pct}%` }}>
            <div className={`w-px ${labelled ? 'h-full bg-daw-border' : 'h-2 mt-7 bg-daw-line'}`} />
            {labelled && (
              <span className="absolute top-1 left-1.5 font-lcd text-[9.5px] text-[#7b766c] tabular-nums">
                {i + 1}
              </span>
            )}
          </div>
        );
      })}
      {/* Playhead */}
      <div
        ref={headRef}
        className="playhead absolute inset-y-0 pointer-events-none"
        style={{ left: 0 }}
      />
    </div>
  );
}
