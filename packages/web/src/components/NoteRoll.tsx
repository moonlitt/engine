import { useEffect, useRef } from 'react';
import { useProjectStore } from '../stores/project';
import { useSessionStore } from '../stores/session';
import { useMetersStore } from '../stores/meters';
import { isTauriRuntime } from '../services/transport';

/**
 * Piano-roll preview — every note of the loaded clip drawn under the
 * ruler, coloured by channel, with the playhead gliding across.
 *
 * Render strategy mirrors the meters: the note field is painted ONCE
 * per clip/resize onto a canvas; the playhead is a separate
 * absolutely-positioned div updated imperatively from the 60 Hz meter
 * stream. Nothing re-renders per frame.
 */

/** Same muted DAW palette as the channel colour strips. */
const CHANNEL_COLORS = [
  '#c14d4d', '#cf8a3c', '#c9b340', '#5c9a5c',
  '#4a9090', '#5a9ad4', '#9aa0a6', '#b8688f',
];

type NoteTuple = [number, number, number, number, number];

const ROLL_HEIGHT = 148;

export function NoteRoll() {
  const midi = useProjectStore((s) => s.midi);
  const send = useSessionStore((s) => s.send);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const headRef = useRef<HTMLDivElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const notesRef = useRef<NoteTuple[] | null>(null);

  const total = midi?.totalTicks ?? 0;
  const midiName = midi?.name ?? null;

  // Fetch + paint when the clip changes (and on resize).
  useEffect(() => {
    if (!midi || total <= 0 || !isTauriRuntime()) return;
    let cancelled = false;

    const paint = () => {
      const canvas = canvasRef.current;
      const notes = notesRef.current;
      if (!canvas || !notes) return;
      const cssWidth = canvas.clientWidth;
      const dpr = window.devicePixelRatio || 1;
      canvas.width = Math.round(cssWidth * dpr);
      canvas.height = Math.round(ROLL_HEIGHT * dpr);
      const ctx = canvas.getContext('2d');
      if (!ctx) return;
      ctx.scale(dpr, dpr);
      ctx.clearRect(0, 0, cssWidth, ROLL_HEIGHT);

      if (notes.length === 0) return;
      // Auto-fit the pitch range, padded, at least two octaves so
      // sparse clips don't blow single notes up into bars.
      let lo = 127;
      let hi = 0;
      for (const n of notes) {
        if (n[1] < lo) lo = n[1];
        if (n[1] > hi) hi = n[1];
      }
      lo = Math.max(0, lo - 2);
      hi = Math.min(127, hi + 2);
      if (hi - lo < 24) {
        const pad = Math.ceil((24 - (hi - lo)) / 2);
        lo = Math.max(0, lo - pad);
        hi = Math.min(127, hi + pad);
      }
      const span = hi - lo + 1;
      const rowH = ROLL_HEIGHT / span;
      const noteH = Math.max(1.5, rowH * 0.8);

      // Octave guide lines (every C) keep the field readable.
      ctx.fillStyle = 'rgba(255,255,255,0.045)';
      for (let key = lo; key <= hi; key++) {
        if (key % 12 === 0) {
          const y = ROLL_HEIGHT - ((key - lo + 1) / span) * ROLL_HEIGHT;
          ctx.fillRect(0, y, cssWidth, 1);
        }
      }

      for (const [ch, key, start, dur, vel] of notes) {
        const x = (start / total) * cssWidth;
        const w = Math.max(1.5, (dur / total) * cssWidth);
        const y = ROLL_HEIGHT - ((key - lo + 1) / span) * ROLL_HEIGHT;
        ctx.globalAlpha = 0.45 + (vel / 127) * 0.55;
        ctx.fillStyle = CHANNEL_COLORS[ch % CHANNEL_COLORS.length];
        ctx.fillRect(x, y, w, noteH);
      }
      ctx.globalAlpha = 1;
    };

    void (async () => {
      try {
        const core = await import('@tauri-apps/api/core');
        const notes = await core.invoke<NoteTuple[]>('cmd_midi_notes');
        if (cancelled) return;
        notesRef.current = notes;
        paint();
      } catch (err) {
        console.error('[note-roll]', err);
      }
    })();

    const onResize = () => paint();
    window.addEventListener('resize', onResize);
    return () => {
      cancelled = true;
      window.removeEventListener('resize', onResize);
    };
    // midiName proxies "a different clip was loaded".
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [midiName, total]);

  // Imperative playhead from the meter stream.
  useEffect(() => {
    if (!midi || total <= 0) return;
    const draw = (ticks: number) => {
      if (headRef.current) {
        headRef.current.style.left = `${Math.min(100, (ticks / total) * 100)}%`;
      }
    };
    draw(useMetersStore.getState().playheadTicks);
    return useMetersStore.subscribe((s) => draw(s.playheadTicks));
  }, [midi, total]);

  if (!midi || total <= 0 || !isTauriRuntime()) return null;

  return (
    <div
      ref={wrapRef}
      className="relative shrink-0 border-b border-black/50 bg-[#141312] cursor-pointer"
      style={{ height: ROLL_HEIGHT }}
      onPointerDown={(e) => {
        const rect = (e.currentTarget as HTMLDivElement).getBoundingClientRect();
        const frac = Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width));
        send({ type: 'transport.seek', ticks: frac * total });
      }}
      title="音符卷帘 — 点击跳转播放位置"
    >
      <canvas ref={canvasRef} className="w-full h-full block" />
      <div
        ref={headRef}
        className="absolute inset-y-0 w-px bg-daw-accent-hi/80 pointer-events-none"
        style={{ left: 0 }}
      />
    </div>
  );
}
