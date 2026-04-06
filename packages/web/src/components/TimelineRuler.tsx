import { useMemo } from 'react';

interface TimelineRulerProps {
  pixelsPerBar: number;
  totalBars: number;
  scrollLeft: number;
}

export function TimelineRuler({ pixelsPerBar, totalBars, scrollLeft }: TimelineRulerProps) {
  const bars = useMemo(() => {
    const result: number[] = [];
    for (let i = 1; i <= totalBars; i++) {
      result.push(i);
    }
    return result;
  }, [totalBars]);

  return (
    <div className="h-6 bg-daw-panel border-b border-daw-border flex items-end overflow-hidden relative">
      {/* Spacer matching track header width */}
      <div className="w-[120px] shrink-0 border-r border-daw-border" />

      {/* Bar numbers - scrollable area */}
      <div className="flex-1 relative h-full" style={{ overflow: 'hidden' }}>
        <div
          className="absolute top-0 left-0 h-full flex items-end"
          style={{ transform: `translateX(-${scrollLeft}px)` }}
        >
          {bars.map((bar) => (
            <div
              key={bar}
              className="h-full flex items-end border-l border-daw-border/50"
              style={{ width: `${pixelsPerBar}px` }}
            >
              <span className="text-[10px] text-[#666] pl-1 pb-0.5 select-none">
                {bar}
              </span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
