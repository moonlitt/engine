import { create } from 'zustand';
import type { Command } from '@moonlitt/protocol';
import { getTransport } from '../services/transport';

interface SessionStore {
  connected: boolean;
  send(command: Command): void;
  setConnected(connected: boolean): void;
}

export const useSessionStore = create<SessionStore>((set) => ({
  connected: false,
  send(command: Command) {
    getTransport().send(command);
  },
  setConnected(connected: boolean) {
    set({ connected });
  },
}));
