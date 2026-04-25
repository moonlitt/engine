import { create } from 'zustand';
import type { PluginInfo } from '@moonlitt/protocol';

interface PluginsStore {
  list: PluginInfo[];
  scanning: boolean;
  setList(plugins: PluginInfo[]): void;
  setScanning(scanning: boolean): void;
}

export const usePluginsStore = create<PluginsStore>((set) => ({
  list: [],
  scanning: false,
  setList(list: PluginInfo[]) {
    set({ list, scanning: false });
  },
  setScanning(scanning: boolean) {
    set({ scanning });
  },
}));
