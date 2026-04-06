import { create } from 'zustand';

interface TransportStore {
  playing: boolean;
  bpm: number;
  position: number;
  timeSignature: [number, number];
  setPlaying(playing: boolean): void;
  setBpm(bpm: number): void;
  updatePosition(pos: number): void;
  setTimeSignature(sig: [number, number]): void;
}

export const useTransportStore = create<TransportStore>((set) => ({
  playing: false,
  bpm: 120,
  position: 0,
  timeSignature: [4, 4],

  setPlaying(playing: boolean) {
    set({ playing });
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
