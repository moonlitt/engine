import { create } from 'zustand';

interface UiStore {
  /** Track currently targeted by the instrument-selector modal, or null when closed. */
  instrumentSelectorTrackId: number | null;
  openInstrumentSelector(trackId: number): void;
  closeInstrumentSelector(): void;
}

export const useUiStore = create<UiStore>((set) => ({
  instrumentSelectorTrackId: null,
  openInstrumentSelector(trackId: number) {
    set({ instrumentSelectorTrackId: trackId });
  },
  closeInstrumentSelector() {
    set({ instrumentSelectorTrackId: null });
  },
}));
