import { create } from 'zustand';
import type { Command } from '@moonlitt/protocol';

interface SessionStore {
  connected: boolean;
  ws: WebSocket | null;
  connect(url: string): void;
  send(command: Command): void;
  disconnect(): void;
  setConnected(connected: boolean): void;
  setWs(ws: WebSocket | null): void;
}

export const useSessionStore = create<SessionStore>((set, get) => ({
  connected: false,
  ws: null,

  connect(url: string) {
    const existing = get().ws;
    if (existing) {
      existing.close();
    }

    const ws = new WebSocket(url);
    ws.binaryType = 'arraybuffer';

    ws.addEventListener('open', () => {
      set({ connected: true });
    });

    ws.addEventListener('close', () => {
      set({ connected: false, ws: null });
    });

    ws.addEventListener('error', () => {
      set({ connected: false });
    });

    set({ ws });
  },

  send(command: Command) {
    const { ws, connected } = get();
    if (ws && connected) {
      ws.send(JSON.stringify(command));
    }
  },

  disconnect() {
    const { ws } = get();
    if (ws) {
      ws.close();
    }
    set({ ws: null, connected: false });
  },

  setConnected(connected: boolean) {
    set({ connected });
  },

  setWs(ws: WebSocket | null) {
    set({ ws });
  },
}));
