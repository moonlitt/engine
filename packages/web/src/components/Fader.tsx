import { useRef, useCallback } from 'react';

interface FaderProps {
  value: number; // dB
  onChange: (db: number) => void;
  min?: number;
  max?: number;
  height?: number;
}

function dbToDisplay(db: number, min: number): string {
  if (db <= min) return '-inf';
  return db.toFixed(1);
}

function dbToPercent(db: number, min: number, max: number): number {
  if (db <= min) return 0;
  return (db - min) / (max - min);
}

function percentToDb(pct: number, min: number, max: number): number {
  const db = min + pct * (max - min);
  if (db <= min) return -Infinity;
  return Math.round(db * 10) / 10;
}

export function Fader({
  value,
  onChange,
  min = -60,
  max = 6,
  height = 100,
}: FaderProps) {
  const trackRef = useRef<HTMLDivElement>(null);

  const handlePointerDown = useCallback(
    (e: React.PointerEvent) => {
      e.preventDefault();
      const target = e.currentTarget as HTMLElement;
      target.setPointerCapture(e.pointerId);

      function update(clientY: number) {
        const track = trackRef.current;
        if (!track) return;
        const rect = track.getBoundingClientRect();
        // Invert: top = max, bottom = min
        const pct = 1 - Math.max(0, Math.min(1, (clientY - rect.top) / rect.height));
        onChange(percentToDb(pct, min, max));
      }

      update(e.clientY);

      function onMove(ev: PointerEvent) {
        update(ev.clientY);
      }
      function onUp() {
        target.removeEventListener('pointermove', onMove);
        target.removeEventListener('pointerup', onUp);
      }
      target.addEventListener('pointermove', onMove);
      target.addEventListener('pointerup', onUp);
    },
    [min, max, onChange],
  );

  const pct = dbToPercent(value, min, max);
  const thumbY = (1 - pct) * height;

  return (
    <div className="flex flex-col items-center gap-1">
      <div
        ref={trackRef}
        className="relative bg-daw-bg rounded cursor-ns-resize"
        style={{ width: 10, height }}
        onPointerDown={handlePointerDown}
      >
        {/* Track fill */}
        <div
          className="absolute bottom-0 left-0 w-full rounded bg-daw-accent/40"
          style={{ height: `${pct * 100}%` }}
        />
        {/* Thumb */}
        <div
          className="absolute left-1/2 -translate-x-1/2 w-4 h-2 bg-[#ccc] rounded-sm border border-[#888]"
          style={{ top: thumbY - 4 }}
        />
      </div>
      <span className="text-[10px] text-[#888] select-none tabular-nums">
        {dbToDisplay(value, min)}
      </span>
    </div>
  );
}
