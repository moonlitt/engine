import { create } from 'zustand';

interface TransportStore {
  playing: boolean;
  looping: boolean;
  metronomeEnabled: boolean;
  bpm: number;
  position: number;
  timeSignature: [number, number];
  setPlaying(playing: boolean): void;
  setLooping(looping: boolean): void;
  setMetronomeEnabled(enabled: boolean): void;
  setBpm(bpm: number): void;
  updatePosition(pos: number): void;
  setTimeSignature(sig: [number, number]): void;
}

export const useTransportStore = create<TransportStore>((set) => ({
  playing: false,
  looping: false,
  metronomeEnabled: false,
  bpm: 120,
  position: 0,
  timeSignature: [4, 4],

  setPlaying(playing: boolean) {
    set({ playing });
  },

  setLooping(looping: boolean) {
    set({ looping });
  },

  setMetronomeEnabled(enabled: boolean) {
    set({ metronomeEnabled: enabled });
  },

  setBpm(bpm: number) {
    set({ bpm });
  },

  updatePosition(pos: number) {
    set({ position: pos });
  },

  setTimeSignature(sig: [number, number]) {
    set({ timeSignature: sig });
  },
}));
