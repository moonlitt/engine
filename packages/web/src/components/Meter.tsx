import { useEffect, useRef } from 'react';
import { useMetersStore, type StereoMeter } from '../stores/meters';

interface MeterProps {
  /** `null` → master meter; `0..15` → MIDI channel override meter. */
  channel: number | null;
  /** CSS pixel width of each bar. */
  width?: number;
  /** CSS pixel height of each bar. */
  height?: number;
  /** Vertical gap between L and R bars (defaults to 1 px). */
  gap?: number;
  className?: string;
}

const PEAK_HOLD_MS = 1500;
const PEAK_FALL_DB_PER_S = 18;
const FLOOR_DB = -60;

function linearToFraction(linear: number): number {
  if (linear <= 0) return 0;
  const db = 20 * Math.log10(linear);
  if (db <= FLOOR_DB) return 0;
  if (db >= 0) return 1;
  return (db - FLOOR_DB) / -FLOOR_DB;
}

function fractionToDb(fraction: number): number {
  return fraction * -FLOOR_DB + FLOOR_DB;
}

function decayPeak(peak: number, peakAt: number, now: number): number {
  if (peak <= 0) return 0;
  const elapsed = now - peakAt;
  if (elapsed <= PEAK_HOLD_MS) return peak;
  const seconds = (elapsed - PEAK_HOLD_MS) / 1000;
  const fall = 1 - seconds * (PEAK_FALL_DB_PER_S / -FLOOR_DB);
  return Math.max(0, peak * fall);
}

/**
 * DAW-style peak meter. Canvas-drawn so 60 Hz updates don't trigger React
 * re-renders. Subscribes imperatively to {@link useMetersStore} and falls
 * back to a continuous animation frame when no events arrive, so peak-hold
 * decay stays smooth even during silence.
 */
export function Meter({
  channel,
  width = 72,
  height = 5,
  gap = 1,
  className,
}: MeterProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (canvas === null) return;
    const ctx = canvas.getContext('2d');
    if (ctx === null) return;

    const dpr = window.devicePixelRatio || 1;
    const totalHeight = height * 2 + gap;
    canvas.width = width * dpr;
    canvas.height = totalHeight * dpr;
    canvas.style.width = `${width}px`;
    canvas.style.height = `${totalHeight}px`;
    ctx.scale(dpr, dpr);

    let peakL = 0;
    let peakLAt = 0;
    let peakR = 0;
    let peakRAt = 0;
    let rafId = 0;

    function draw(now: number, m: StereoMeter) {
      if (ctx === null) return;
      // Update peak hold based on the latest meter sample.
      if (m.l > peakL) { peakL = m.l; peakLAt = now; }
      else peakL = decayPeak(peakL, peakLAt, now);
      if (m.r > peakR) { peakR = m.r; peakRAt = now; }
      else peakR = decayPeak(peakR, peakRAt, now);

      // Clear.
      ctx.clearRect(0, 0, width, totalHeight);

      drawBar(ctx, 0, m.l, peakL);
      drawBar(ctx, height + gap, m.r, peakR);
    }

    function drawBar(ctx: CanvasRenderingContext2D, y: number, level: number, peak: number) {
      // Track background.
      ctx.fillStyle = '#0e0e0e';
      ctx.fillRect(0, y, width, height);

      const f = linearToFraction(level);
      const filled = f * width;
      if (filled > 0) {
        // Gradient: dim-green up to -18 dB, bright-green to -6 dB,
        // amber to -3 dB, orange to 0 dB, red above 0 dB.
        const grad = ctx.createLinearGradient(0, 0, width, 0);
        grad.addColorStop(0.0, '#2d5a3f');                   // -60 dB dim green
        grad.addColorStop(linearToFraction(0.125), '#3d8a5e'); // -18 dB
        grad.addColorStop(linearToFraction(0.5), '#5fbf7e');   //  -6 dB bright
        grad.addColorStop(linearToFraction(0.708), '#d4a017'); //  -3 dB amber
        grad.addColorStop(0.995, '#e35d2a');                  //   0 dB orange
        grad.addColorStop(1.0, '#e84a2a');                    //   clip red
        ctx.fillStyle = grad;
        ctx.fillRect(0, y, filled, height);
      }

      // Peak hold tick — 1 px bright line.
      if (peak > 0) {
        const px = Math.min(width - 1, Math.floor(linearToFraction(peak) * width));
        ctx.fillStyle = fractionToDb(linearToFraction(peak)) >= -0.05 ? '#ff5a3a' : '#d8d8d8';
        ctx.fillRect(px, y, 1, height);
      }
    }

    function tick(now: number) {
      const s = useMetersStore.getState();
      const m = channel === null ? s.master : s.tracks[channel] ?? { l: 0, r: 0 };
      draw(now, m);
      rafId = requestAnimationFrame(tick);
    }

    rafId = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafId);
  }, [channel, width, height, gap]);

  return <canvas ref={canvasRef} className={className} />;
}
