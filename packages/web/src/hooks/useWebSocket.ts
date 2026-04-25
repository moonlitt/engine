import { useEffect, useRef } from 'react';
import type { ServerEvent } from '@moonlitt/protocol';
import { useSessionStore } from '../stores/session';
import { useTransportStore } from '../stores/transport';
import { useMixerStore } from '../stores/mixer';
import { usePluginsStore } from '../stores/plugins';

const WS_URL = 'ws://localhost:3001';
const RECONNECT_DELAY_MS = 2000;

export function useWebSocket(): void {
  const reconnectTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const intentionalClose = useRef(false);

  useEffect(() => {
    function connect() {
      intentionalClose.current = false;
      const ws = new WebSocket(WS_URL);
      ws.binaryType = 'arraybuffer';

      ws.addEventListener('open', () => {
        useSessionStore.getState().setWs(ws);
        useSessionStore.getState().setConnected(true);
      });

      ws.addEventListener('message', (event) => {
        if (event.data instanceof ArrayBuffer) {
          handleBinaryMessage(event.data);
        } else {
          handleJsonMessage(event.data as string);
        }
      });

      ws.addEventListener('close', () => {
        useSessionStore.getState().setConnected(false);
        useSessionStore.getState().setWs(null);

        if (!intentionalClose.current) {
          scheduleReconnect();
        }
      });

      ws.addEventListener('error', () => {
        // The close event will fire after error, triggering reconnect
      });
    }

    function scheduleReconnect() {
      if (reconnectTimer.current !== null) {
        return;
      }
      reconnectTimer.current = setTimeout(() => {
        reconnectTimer.current = null;
        connect();
      }, RECONNECT_DELAY_MS);
    }

    connect();

    return () => {
      intentionalClose.current = true;
      if (reconnectTimer.current !== null) {
        clearTimeout(reconnectTimer.current);
        reconnectTimer.current = null;
      }
      const ws = useSessionStore.getState().ws;
      if (ws) {
        ws.close();
      }
    };
  }, []);
}

function handleJsonMessage(raw: string): void {
  let event: ServerEvent;
  try {
    event = JSON.parse(raw) as ServerEvent;
  } catch {
    return;
  }

  switch (event.type) {
    case 'state.init': {
      useMixerStore.getState().initTracks(event.tracks);
      useTransportStore.getState().setBpm(event.bpm);
      useTransportStore.getState().setPlaying(event.playing);
      break;
    }
    case 'track.added': {
      useMixerStore.getState().addTrack(event.trackId, event.name, event.color);
      break;
    }
    case 'track.removed': {
      useMixerStore.getState().removeTrack(event.trackId);
      break;
    }
    case 'transport.state': {
      useTransportStore.getState().setPlaying(event.playing);
      useTransportStore.getState().updatePosition(event.position);
      break;
    }
    case 'midi.clip_added': {
      useMixerStore.getState().addClip(event.trackId, event.clip);
      break;
    }
    case 'track.instrument_changed': {
      useMixerStore.getState().setTrackInstrument(event.trackId, event.instrumentPath);
      break;
    }
    case 'insert.added': {
      useMixerStore.getState().addInsert(event.trackId, event.insert);
      break;
    }
    case 'insert.removed': {
      useMixerStore.getState().removeInsert(event.trackId, event.insertId);
      break;
    }
    case 'plugins.list': {
      usePluginsStore.getState().setList(event.plugins);
      break;
    }
    case 'error': {
      // Future: dispatch to a notification/toast store
      break;
    }
  }
}

function handleBinaryMessage(buffer: ArrayBuffer): void {
  const data = new Float32Array(buffer);
  useMixerStore.getState().updateMeters(data);
}
