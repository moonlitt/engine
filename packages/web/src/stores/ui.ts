import { create } from 'zustand';

/**
 * Target the instrument-picker modal applies to:
 *   { kind: 'default' }              → set the project's default instrument
 *   { kind: 'override', channel: N } → set / replace the channel-N override
 */
export type InstrumentTarget =
  | { kind: 'default' }
  | { kind: 'override'; channel: number };

interface UiStore {
  instrumentTarget: InstrumentTarget | null;
  openInstrumentPicker(target: InstrumentTarget): void;
  closeInstrumentPicker(): void;

  /** Target the STEAM patch-library browser applies to (Spectrasonics
   *  instruments only); null = closed. */
  patchBrowserTarget: InstrumentTarget | null;
  openPatchBrowser(target: InstrumentTarget): void;
  closePatchBrowser(): void;
}

export const useUiStore = create<UiStore>((set) => ({
  instrumentTarget: null,
  openInstrumentPicker(target) {
    set({ instrumentTarget: target });
  },
  closeInstrumentPicker() {
    set({ instrumentTarget: null });
  },

  patchBrowserTarget: null,
  openPatchBrowser(target) {
    set({ patchBrowserTarget: target });
  },
  closePatchBrowser() {
    set({ patchBrowserTarget: null });
  },
}));
