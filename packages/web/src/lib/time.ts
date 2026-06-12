import type { MidiState } from '@moonlitt/protocol';

/**
 * Tick → musical/clock readouts for the LCD and ruler. All math comes
 * from the loaded clip's own resolution and time signature; tempo is
 * the file's initial BPM (good enough for a readout — the sequencer
 * itself follows the full tempo map).
 */

export interface ClipTime {
  ticksPerBar: number;
  totalBars: number;
}

export function clipTime(midi: MidiState): ClipTime {
  const tpb = midi.ticksPerBeat > 0 ? midi.ticksPerBeat : 480;
  const [num, den] = midi.timeSignature ?? [4, 4];
  const ticksPerBar = tpb * num * (4 / den);
  const totalBars = Math.max(1, Math.ceil(midi.totalTicks / ticksPerBar));
  return { ticksPerBar, totalBars };
}

/** "5.3" — bar.beat (both 1-based), Logic LCD style. */
export function formatBarsBeats(ticks: number, midi: MidiState): string {
  const tpb = midi.ticksPerBeat > 0 ? midi.ticksPerBeat : 480;
  const [num, den] = midi.timeSignature ?? [4, 4];
  const ticksPerBar = tpb * num * (4 / den);
  const bar = Math.floor(ticks / ticksPerBar) + 1;
  const beat = Math.floor((ticks % ticksPerBar) / (tpb * (4 / den))) + 1;
  return `${bar}.${beat}`;
}

/** "01:23.456" — minutes:seconds.millis from the file's initial tempo. */
export function formatClock(ticks: number, midi: MidiState): string {
  const tpb = midi.ticksPerBeat > 0 ? midi.ticksPerBeat : 480;
  const bpm = midi.tempoBpm && midi.tempoBpm > 0 ? midi.tempoBpm : 120;
  const seconds = Math.max(0, (ticks / tpb) * (60 / bpm));
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  const ms = Math.floor((seconds - Math.floor(seconds)) * 1000);
  return `${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}.${String(ms).padStart(3, '0')}`;
}
