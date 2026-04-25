import { useEffect } from 'react';
import { useSessionStore } from '../stores/session';
import { getTransport } from '../services/transport';

/**
 * Connects the active transport (WebSocket or Tauri IPC) to the session
 * store. Name kept for legacy reasons; works for both transports.
 */
export function useWebSocket(): void {
  useEffect(() => {
    const transport = getTransport();
    const unsub = transport.onConnectionChange((c) => {
      useSessionStore.getState().setConnected(c);
    });
    void Promise.resolve(transport.start());
    return () => {
      unsub();
      transport.stop();
    };
  }, []);
}
