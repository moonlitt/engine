import { create } from 'zustand';

/**
 * Live peak meters store, fed from the backend's `meter` event at ~60 Hz.
 *
 * Components don't subscribe via React selector hooks (that would re-render
 * every channel meter every 16 ms). Instead the `Meter` component subscribes
 * imperatively (`useMetersStore.subscribe(...)`) and draws to canvas, so the
 * React tree stays still during playback.
 */

export interface StereoMeter {
  l: number;
  r: number;
}

export interface MeterSnapshot {
  master: [number, number];
  tracks: Array<{ channel: number; l: number; r: number }>;
}

interface MetersStore {
  master: StereoMeter;
  /** MIDI channel (0..15) → stereo peak for the corresponding override track. */
  tracks: Record<number, StereoMeter>;
  apply(snapshot: MeterSnapshot): void;
}

const EMPTY: StereoMeter = { l: 0, r: 0 };

export const useMetersStore = create<MetersStore>((set) => ({
  master: EMPTY,
  tracks: {},
  apply(snap) {
    const tracks: Record<number, StereoMeter> = {};
    for (const t of snap.tracks) {
      tracks[t.channel] = { l: t.l, r: t.r };
    }
    set({
      master: { l: snap.master[0], r: snap.master[1] },
      tracks,
    });
  },
}));

/** Convenience: read a meter once without subscribing. */
export function readMeter(channel: number | null): StereoMeter {
  const s = useMetersStore.getState();
  if (channel === null) return s.master;
  return s.tracks[channel] ?? EMPTY;
}
