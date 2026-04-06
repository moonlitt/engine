import { useRef, useEffect } from 'react';
import { useMixerStore } from '../stores/mixer';

interface MeterProps {
  trackId: number | 'master';
  width?: number;
  height?: number;
}

function drawBar(
  ctx: CanvasRenderingContext2D,
  x: number,
  w: number,
  h: number,
  peak: number,
) {
  const barH = peak * h;

  // Green zone (0-70%)
  const greenH = Math.min(barH, h * 0.7);
  ctx.fillStyle = '#4caf50';
  ctx.fillRect(x, h - greenH, w, greenH);

  // Yellow zone (70-90%)
  if (barH > h * 0.7) {
    const yellowH = Math.min(barH - h * 0.7, h * 0.2);
    ctx.fillStyle = '#ffeb3b';
    ctx.fillRect(x, h - h * 0.7 - yellowH, w, yellowH);
  }

  // Red zone (>90%)
  if (barH > h * 0.9) {
    const redH = barH - h * 0.9;
    ctx.fillStyle = '#f44336';
    ctx.fillRect(x, h - h * 0.9 - redH, w, redH);
  }
}

export function Meter({ trackId, width = 20, height = 100 }: MeterProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const peakRef = useRef({ l: 0, r: 0 });

  // Subscribe directly to store updates, bypassing React re-renders
  useEffect(() => {
    const unsub = useMixerStore.subscribe((state, prev) => {
      if (trackId === 'master') {
        if (
          state.masterPeakL !== prev.masterPeakL ||
          state.masterPeakR !== prev.masterPeakR
        ) {
          peakRef.current = { l: state.masterPeakL, r: state.masterPeakR };
        }
      } else {
        const track = state.tracks.find((t) => t.id === trackId);
        const prevTrack = prev.tracks.find((t) => t.id === trackId);
        if (track && (track.peakL !== prevTrack?.peakL || track.peakR !== prevTrack?.peakR)) {
          peakRef.current = { l: track.peakL, r: track.peakR };
        }
      }
    });
    return unsub;
  }, [trackId]);

  // rAF draw loop -- independent of React render cycle
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    let animId: number;

    function draw() {
      const { l, r } = peakRef.current;
      ctx!.clearRect(0, 0, width, height);

      const barW = (width - 2) / 2;
      drawBar(ctx!, 0, barW, height, l);
      drawBar(ctx!, barW + 2, barW, height, r);

      animId = requestAnimationFrame(draw);
    }

    animId = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(animId);
  }, [width, height]);

  return <canvas ref={canvasRef} width={width} height={height} className="block" />;
}
