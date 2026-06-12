import { useEffect, useRef, useState } from 'react';
import { useProjectStore } from '../stores/project';
import { useSessionStore } from '../stores/session';
import { useTransportStore } from '../stores/transport';
import { useMetersStore } from '../stores/meters';
import { clipTime } from '../lib/time';

/**
 * Bar ruler + playhead + progress, full width under the transport.
 *
 * - Click / drag: seek (audio thread releases held notes and jumps).
 * - ⌥-drag: draw a practice-loop region (snapped to bars) — looping
 *   turns on automatically. Double-click the region band to clear it.
 *
 * Playhead and drag preview are positioned imperatively from the 60 Hz
 * meter stream / pointer events so the React tree stays still.
 */
export function Ruler() {
  const midi = useProjectStore((s) => s.midi);
  const send = useSessionStore((s) => s.send);
  const loopRegion = useTransportStore((s) => s.loopRegion);
  const laneRef = useRef<HTMLDivElement>(null);
  const headRef = useRef<HTMLDivElement>(null);
  const fillRef = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);
  /** ⌥-drag in progress: anchor tick of the region being drawn. */
  const regionAnchor = useRef<number | null>(null);
  const [draftRegion, setDraftRegion] = useState<[number, number] | null>(null);

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

  const tickFromPointer = (clientX: number): number => {
    const lane = laneRef.current;
    if (!lane) return 0;
    const rect = lane.getBoundingClientRect();
    const frac = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
    return frac * total;
  };

  /** Snap a tick to the nearest bar line — practice loops are bars. */
  const snapToBar = (tick: number): number =>
    Math.min(total, Math.max(0, Math.round(tick / ticksPerBar) * ticksPerBar));

  const region = draftRegion ?? loopRegion;

  return (
    <div
      ref={laneRef}
      className="ruler relative h-9 shrink-0 border-b border-black/50 overflow-hidden"
      onPointerDown={(e) => {
        (e.target as Element).setPointerCapture?.(e.pointerId);
        if (e.altKey) {
          regionAnchor.current = snapToBar(tickFromPointer(e.clientX));
          setDraftRegion(null);
        } else {
          dragging.current = true;
          send({ type: 'transport.seek', ticks: tickFromPointer(e.clientX) });
        }
      }}
      onPointerMove={(e) => {
        if (regionAnchor.current !== null) {
          const a = regionAnchor.current;
          const b = snapToBar(tickFromPointer(e.clientX));
          setDraftRegion(a === b ? null : [Math.min(a, b), Math.max(a, b)]);
        } else if (dragging.current) {
          send({ type: 'transport.seek', ticks: tickFromPointer(e.clientX) });
        }
      }}
      onPointerUp={() => {
        dragging.current = false;
        if (regionAnchor.current !== null) {
          regionAnchor.current = null;
          setDraftRegion(null);
          if (draftRegion) {
            send({
              type: 'transport.set_loop_region',
              startTicks: draftRegion[0],
              endTicks: draftRegion[1],
            });
          }
        }
      }}
      onDoubleClick={() => {
        if (loopRegion) {
          send({ type: 'transport.set_loop_region', startTicks: null, endTicks: null });
        }
      }}
      title="点击/拖拽：跳转播放位置 · ⌥拖拽：设置循环区间（按小节吸附） · 双击：清除区间"
    >
      {/* Elapsed-region tint */}
      <div
        ref={fillRef}
        className="absolute inset-y-0 left-0 bg-daw-accent/[0.07] pointer-events-none"
        style={{ width: 0 }}
      />
      {/* Practice-loop region band (Logic's cycle strip) */}
      {region && (
        <div
          className={`absolute top-0 h-[7px] rounded-b-sm pointer-events-none ${
            draftRegion ? 'bg-daw-accent/50' : 'bg-state-solo/70'
          }`}
          style={{
            left: `${(region[0] / total) * 100}%`,
            width: `${((region[1] - region[0]) / total) * 100}%`,
          }}
        />
      )}
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
