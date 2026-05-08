/**
 * Transport abstraction.
 *
 * Picks at module-load time between two implementations that drive the
 * same React stores:
 *   - **WebSocket**: legacy Node server at ws://localhost:3001 (Web DAW)
 *   - **Tauri**: in-process Rust engine via `invoke()` / `event.listen()`
 *
 * Detection uses Tauri 2's official `window.isTauri` marker (same check
 * as `@tauri-apps/api/core`'s `isTauri()`).
 */

import type { Command, ServerEvent } from '@moonlitt/protocol';
import { isTauri } from '@tauri-apps/api/core';
import { useProjectStore } from '../stores/project';
import { useTransportStore } from '../stores/transport';
import { usePluginsStore } from '../stores/plugins';

export interface Transport {
  /** Connection / availability flag; mirrors session.connected. */
  connected(): boolean;
  /** Subscribe to connection changes; returns unsubscribe. */
  onConnectionChange(cb: (c: boolean) => void): () => void;

  /** Send a command. Errors surface via the `error` event channel. */
  send(cmd: Command): void;

  /** Whether the active transport supports drag-dropping File objects. */
  supportsFileDrop: boolean;

  /** Open a native picker (or platform equivalent) and load the chosen MIDI. */
  pickAndLoadMidi(): Promise<boolean>;

  /** Load a MIDI from a File handed in by the UI (e.g. drag-drop). */
  loadMidiFile(file: File): Promise<boolean>;

  /** Initialise (open WebSocket / register Tauri event listeners). */
  start(): Promise<void> | void;
  /** Tear down listeners. */
  stop(): void;
}

let cached: Transport | null = null;

export function isTauriRuntime(): boolean {
  return isTauri();
}

export function getTransport(): Transport {
  if (cached) return cached;
  const tauri = isTauriRuntime();
  // eslint-disable-next-line no-console
  console.log(`[transport] using ${tauri ? 'Tauri IPC' : 'WebSocket'}`);
  cached = tauri ? createTauriTransport() : createWebSocketTransport();
  return cached;
}

// ---------------------------------------------------------------------------
// Event router — both transports funnel into this.
// ---------------------------------------------------------------------------

export function dispatchEvent(event: ServerEvent): void {
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
      console.error('[transport]', event.message);
      break;
  }
}

// ---------------------------------------------------------------------------
// WebSocket implementation (legacy)
// ---------------------------------------------------------------------------

function createWebSocketTransport(): Transport {
  const WS_URL = 'ws://localhost:3001';
  const RECONNECT_DELAY_MS = 2000;
  const API_BASE = 'http://localhost:3001';

  let ws: WebSocket | null = null;
  let intentionalClose = false;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let connected = false;
  const listeners = new Set<(c: boolean) => void>();

  function setConnected(c: boolean): void {
    if (c === connected) return;
    connected = c;
    for (const l of listeners) l(c);
  }

  function connect() {
    intentionalClose = false;
    ws = new WebSocket(WS_URL);
    ws.binaryType = 'arraybuffer';

    ws.addEventListener('open', () => setConnected(true));
    ws.addEventListener('close', () => {
      setConnected(false);
      ws = null;
      if (!intentionalClose) scheduleReconnect();
    });
    ws.addEventListener('message', (e) => {
      if (e.data instanceof ArrayBuffer) return; // meter binary frames
      try {
        dispatchEvent(JSON.parse(e.data as string) as ServerEvent);
      } catch (err) {
        console.error('[ws] bad message:', err);
      }
    });
    ws.addEventListener('error', () => { /* close fires after */ });
  }

  function scheduleReconnect() {
    if (reconnectTimer !== null) return;
    reconnectTimer = setTimeout(() => {
      reconnectTimer = null;
      connect();
    }, RECONNECT_DELAY_MS);
  }

  async function uploadFile(file: File): Promise<boolean> {
    const fd = new FormData();
    fd.append('file', file);
    fd.append('trackId', '0');
    try {
      const res = await fetch(`${API_BASE}/api/upload-midi`, { method: 'POST', body: fd });
      return res.ok;
    } catch (err) {
      console.error('[ws] upload failed:', err);
      return false;
    }
  }

  return {
    connected: () => connected,
    onConnectionChange(cb) {
      listeners.add(cb);
      cb(connected);
      return () => { listeners.delete(cb); };
    },
    send(cmd) {
      if (ws && connected) ws.send(JSON.stringify(cmd));
    },
    supportsFileDrop: true,
    async pickAndLoadMidi() {
      // Trigger via a hidden input the caller already wires up.
      // (This branch is unused at the moment; WS path uses MidiPanel's
      // own file picker. Kept here so the interface stays uniform.)
      return false;
    },
    loadMidiFile(file) { return uploadFile(file); },
    start() { connect(); },
    stop() {
      intentionalClose = true;
      if (reconnectTimer !== null) {
        clearTimeout(reconnectTimer);
        reconnectTimer = null;
      }
      if (ws) ws.close();
    },
  };
}

// ---------------------------------------------------------------------------
// Tauri implementation
// ---------------------------------------------------------------------------

interface TauriCore {
  invoke<T = unknown>(cmd: string, args?: Record<string, unknown>): Promise<T>;
}
interface TauriEvent {
  listen<T>(event: string, handler: (e: { payload: T }) => void): Promise<() => void>;
}
interface TauriDialog {
  open(opts: { multiple?: boolean; filters?: Array<{ name: string; extensions: string[] }> }): Promise<string | null>;
}

function createTauriTransport(): Transport {
  const listeners = new Set<(c: boolean) => void>();
  let connected = false;
  const unsubs: Array<() => void> = [];

  function setConnected(c: boolean) {
    if (c === connected) return;
    connected = c;
    for (const l of listeners) l(c);
  }

  // Dynamic import so the bundle works in browser builds without Tauri.
  let coreP: Promise<TauriCore> | null = null;
  let eventP: Promise<TauriEvent> | null = null;
  let dialogP: Promise<TauriDialog> | null = null;
  function core(): Promise<TauriCore> {
    coreP ??= import('@tauri-apps/api/core') as Promise<TauriCore>;
    return coreP;
  }
  function evt(): Promise<TauriEvent> {
    eventP ??= import('@tauri-apps/api/event') as Promise<TauriEvent>;
    return eventP;
  }
  function dialog(): Promise<TauriDialog> {
    dialogP ??= import('@tauri-apps/plugin-dialog') as Promise<TauriDialog>;
    return dialogP;
  }

  async function invoke<T = unknown>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    const c = await core();
    return c.invoke<T>(cmd, args);
  }

  function send(cmd: Command): void {
    void dispatchCommand(cmd).catch((err) => {
      console.error(`[tauri] ${cmd.type}:`, err);
    });
  }

  async function dispatchCommand(cmd: Command): Promise<void> {
    switch (cmd.type) {
      case 'transport.play':
        await invoke('cmd_transport_play');
        return;
      case 'transport.stop':
        await invoke('cmd_transport_stop');
        return;
      case 'transport.set_bpm':
        await invoke('cmd_transport_set_bpm', { bpm: cmd.bpm });
        return;
      case 'master.set_volume':
        await invoke('cmd_master_set_volume', { db: cmd.db });
        return;
      case 'plugins.scan':
        await invoke('cmd_plugins_scan', { force: cmd.force ?? false });
        return;
      case 'default.set_instrument':
        await invoke('cmd_default_set_instrument', { path: cmd.path });
        return;
      case 'channel.set_override':
        await invoke('cmd_channel_set_override', { channel: cmd.channel, path: cmd.path });
        return;
      case 'channel.remove_override':
        await invoke('cmd_channel_remove_override', { channel: cmd.channel });
        return;
      case 'channel.set_volume':
        await invoke('cmd_channel_set_volume', { channel: cmd.channel, db: cmd.db });
        return;
      case 'channel.set_mute':
        await invoke('cmd_channel_set_mute', { channel: cmd.channel, muted: cmd.muted });
        return;
      case 'channel.set_solo':
        await invoke('cmd_channel_set_solo', { channel: cmd.channel, solo: cmd.solo });
        return;
      case 'channel.set_program':
        await invoke('cmd_channel_set_program', { channel: cmd.channel, program: cmd.program });
        return;
      case 'insert.add':
        await invoke('cmd_insert_add', { channel: cmd.channel, effectType: cmd.effectType });
        return;
      case 'insert.remove':
        await invoke('cmd_insert_remove', { channel: cmd.channel, insertId: cmd.insertId });
        return;
      case 'insert.set_param':
        await invoke('cmd_insert_set_param', {
          channel: cmd.channel,
          insertId: cmd.insertId,
          paramId: cmd.paramId,
          value: cmd.value,
        });
        return;
    }
  }

  async function loadMidiByPath(path: string): Promise<boolean> {
    try {
      await invoke('cmd_load_midi', { path });
      return true;
    } catch (err) {
      console.error('[tauri] cmd_load_midi:', err);
      return false;
    }
  }

  return {
    connected: () => connected,
    onConnectionChange(cb) {
      listeners.add(cb);
      cb(connected);
      return () => { listeners.delete(cb); };
    },
    send,
    supportsFileDrop: false,
    async pickAndLoadMidi() {
      const dlg = await dialog();
      const picked = await dlg.open({
        multiple: false,
        filters: [{ name: 'MIDI', extensions: ['mid', 'midi'] }],
      });
      if (typeof picked !== 'string' || !picked) return false;
      return loadMidiByPath(picked);
    },
    async loadMidiFile(_file) {
      // Dropped-File path is unsupported in Tauri build for now —
      // file blobs would need writing to a temp file via the fs plugin.
      // Use pickAndLoadMidi instead.
      return false;
    },
    async start() {
      const e = await evt();
      // Wire each Tauri event to a synthetic ServerEvent shape.
      type Wrap<T> = { payload: T };
      const onTransportState = await e.listen('transport:state', (m: Wrap<{ playing: boolean; position: number }>) => {
        dispatchEvent({ type: 'transport.state', playing: m.payload.playing, position: m.payload.position });
      });
      const onTempo = await e.listen('transport:tempo_changed', (m: Wrap<{ bpm: number }>) => {
        dispatchEvent({ type: 'transport.tempo_changed', bpm: m.payload.bpm });
      });
      const onMidi = await e.listen('midi:loaded', (m: Wrap<{ midi: import('@moonlitt/protocol').MidiState }>) => {
        dispatchEvent({ type: 'midi.loaded', midi: m.payload.midi });
      });
      const onDefault = await e.listen('default:instrument_changed', (m: Wrap<{ instrumentPath: string | null }>) => {
        dispatchEvent({ type: 'default.instrument_changed', instrumentPath: m.payload.instrumentPath });
      });
      const onAdd = await e.listen('channel:override_added', (m: Wrap<{ override: import('@moonlitt/protocol').ChannelOverrideState }>) => {
        dispatchEvent({ type: 'channel.override_added', override: m.payload.override });
      });
      const onRm = await e.listen('channel:override_removed', (m: Wrap<{ channel: number }>) => {
        dispatchEvent({ type: 'channel.override_removed', channel: m.payload.channel });
      });
      const onUpd = await e.listen('channel:updated', (m: Wrap<{ channel: number; volume?: number; muted?: boolean; solo?: boolean; userProgram?: number | null }>) => {
        dispatchEvent({
          type: 'channel.updated',
          channel: m.payload.channel,
          volume: m.payload.volume,
          muted: m.payload.muted,
          solo: m.payload.solo,
          userProgram: m.payload.userProgram,
        });
      });
      const onIns = await e.listen('insert:added', (m: Wrap<{ channel: number; insert: import('@moonlitt/protocol').InsertState }>) => {
        dispatchEvent({ type: 'insert.added', channel: m.payload.channel, insert: m.payload.insert });
      });
      const onInsRm = await e.listen('insert:removed', (m: Wrap<{ channel: number; insertId: number }>) => {
        dispatchEvent({ type: 'insert.removed', channel: m.payload.channel, insertId: m.payload.insertId });
      });
      const onPlugins = await e.listen('plugins:list', (m: Wrap<{ plugins: import('@moonlitt/protocol').PluginInfo[] }>) => {
        dispatchEvent({ type: 'plugins.list', plugins: m.payload.plugins });
      });
      unsubs.push(
        onTransportState, onTempo, onMidi, onDefault,
        onAdd, onRm, onUpd, onIns, onInsRm, onPlugins,
      );

      // Pull the initial snapshot.
      try {
        const snapshot = await invoke<import('@moonlitt/protocol').ProjectState>('cmd_snapshot');
        dispatchEvent({ type: 'state.init', project: snapshot });
        setConnected(true);
      } catch (err) {
        console.error('[tauri] cmd_snapshot:', err);
        setConnected(false);
      }
    },
    stop() {
      for (const u of unsubs) u();
      unsubs.length = 0;
      setConnected(false);
    },
  };
}
