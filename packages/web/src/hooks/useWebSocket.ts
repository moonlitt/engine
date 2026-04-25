import { useEffect, useRef } from 'react';
import type { ServerEvent } from '@moonlitt/protocol';
import { useSessionStore } from '../stores/session';
import { useTransportStore } from '../stores/transport';
import { useProjectStore } from '../stores/project';
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
          // Binary frames are meter snapshots — not yet wired into the new
          // project store; ignore for now.
          return;
        }
        handleJsonMessage(event.data as string);
      });

      ws.addEventListener('close', () => {
        useSessionStore.getState().setConnected(false);
        useSessionStore.getState().setWs(null);
        if (!intentionalClose.current) scheduleReconnect();
      });

      ws.addEventListener('error', () => {
        // The close event will fire after error, triggering reconnect
      });
    }

    function scheduleReconnect() {
      if (reconnectTimer.current !== null) return;
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
      if (ws) ws.close();
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

  const project = useProjectStore.getState();
  const transport = useTransportStore.getState();

  switch (event.type) {
    case 'state.init':
      project.setProject(event.project);
      transport.setBpm(event.project.bpm);
      transport.setPlaying(event.project.playing);
      break;

    case 'transport.state':
      transport.setPlaying(event.playing);
      transport.updatePosition(event.position);
      break;

    case 'transport.tempo_changed':
      transport.setBpm(event.bpm);
      break;

    case 'midi.loaded':
      project.setMidi(event.midi);
      break;

    case 'default.instrument_changed':
      project.setDefaultInstrument(event.instrumentPath);
      break;

    case 'channel.override_added':
      project.upsertOverride(event.override);
      break;

    case 'channel.override_removed':
      project.removeOverride(event.channel);
      break;

    case 'channel.updated':
      project.updateChannel(event.channel, {
        volume: event.volume,
        muted: event.muted,
        solo: event.solo,
      });
      break;

    case 'insert.added':
      project.addInsert(event.channel, event.insert);
      break;

    case 'insert.removed':
      project.removeInsert(event.channel, event.insertId);
      break;

    case 'plugins.list':
      usePluginsStore.getState().setList(event.plugins);
      break;

    case 'error':
      console.error('[server error]', event.message);
      break;
  }
}
