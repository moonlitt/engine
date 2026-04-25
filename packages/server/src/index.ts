import express from 'express';
import { createServer } from 'http';
import { WebSocketServer, WebSocket } from 'ws';
import multer from 'multer';
import os from 'os';
import { EngineManager } from './engine.js';
import { handleCommand } from './protocol.js';
import type { Command, ServerEvent } from '@moonlitt/protocol';

if (process.env.MOONLITT_SF2_DIR) {
  console.log(`[server] SF2 search dirs (from env): ${process.env.MOONLITT_SF2_DIR}`);
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

const engine = new EngineManager(44100, 512);

if (engine.isAvailable()) {
  console.log('[server] moonlitt-node addon loaded. Audio engine ready.');
} else {
  console.log('[server] Running in UI-only mode (no audio).');
}

// ---------------------------------------------------------------------------
// Express + HTTP
// ---------------------------------------------------------------------------

const app = express();
const server = createServer(app);

app.use((_req, res, next) => {
  res.header('Access-Control-Allow-Origin', '*');
  res.header('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
  res.header('Access-Control-Allow-Headers', 'Content-Type');
  next();
});

const upload = multer({ dest: os.tmpdir() });

app.post('/api/upload-midi', upload.single('file'), (req, res) => {
  if (!req.file) {
    res.status(400).json({ error: 'No file uploaded' });
    return;
  }

  const midi = engine.loadMidi(req.file.path, req.file.originalname);
  if (!midi) {
    res.status(400).json({ error: 'Failed to parse / stage MIDI' });
    return;
  }

  // Broadcast: midi.loaded carries the per-channel info; transport tempo
  // change goes out separately so the UI's BPM display updates.
  broadcast({ type: 'midi.loaded', midi });
  if (midi.tempoBpm !== null && Number.isFinite(midi.tempoBpm)) {
    broadcast({ type: 'transport.tempo_changed', bpm: midi.tempoBpm });
  }

  res.json({ ok: true, channels: midi.channels.map((c) => c.displayNumber) });
});

// ---------------------------------------------------------------------------
// WebSocket
// ---------------------------------------------------------------------------

const wss = new WebSocketServer({ server });

function sendEvent(ws: WebSocket, event: ServerEvent): void {
  if (ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify(event));
  }
}

function broadcast(event: ServerEvent): void {
  const json = JSON.stringify(event);
  for (const ws of wss.clients) {
    if (ws.readyState === WebSocket.OPEN) {
      ws.send(json);
    }
  }
}

wss.on('connection', (ws) => {
  console.log('[server] Client connected');

  const snap = engine.snapshot();
  sendEvent(ws, {
    type: 'state.init',
    project: {
      bpm: snap.bpm,
      playing: snap.playing,
      defaultInstrumentPath: snap.defaultInstrumentPath,
      midi: snap.midi,
      overrides: snap.overrides,
    },
  });

  ws.on('message', (data) => {
    if (typeof data !== 'string' && !(data instanceof Buffer)) return;
    const raw = typeof data === 'string' ? data : data.toString('utf-8');

    let cmd: Command;
    try {
      cmd = JSON.parse(raw) as Command;
    } catch {
      sendEvent(ws, { type: 'error', message: 'Invalid JSON' });
      return;
    }

    if (!cmd.type) {
      sendEvent(ws, { type: 'error', message: 'Missing command type' });
      return;
    }

    const response = handleCommand(engine, cmd);

    if (response === null) return;
    const events = Array.isArray(response) ? response : [response];
    for (const e of events) {
      if (e.type === 'error') {
        sendEvent(ws, e);
      } else {
        broadcast(e);
      }
    }
  });

  ws.on('close', () => {
    console.log('[server] Client disconnected');
  });
});

// ---------------------------------------------------------------------------
// 60fps meter broadcast — binary Float32Array
// Layout: [masterL, masterR, override0_L, override0_R, override1_L, ...]
// ---------------------------------------------------------------------------

const METER_INTERVAL_MS = 16;

const meterTimer = setInterval(() => {
  let hasClient = false;
  for (const ws of wss.clients) {
    if (ws.readyState === WebSocket.OPEN) { hasClient = true; break; }
  }
  if (!hasClient) return;

  const buf = engine.meterSnapshot();
  const bytes = Buffer.from(buf.buffer, buf.byteOffset, buf.byteLength);
  for (const ws of wss.clients) {
    if (ws.readyState === WebSocket.OPEN) {
      ws.send(bytes);
    }
  }
}, METER_INTERVAL_MS);

// ---------------------------------------------------------------------------
// Graceful shutdown
// ---------------------------------------------------------------------------

function shutdown(): void {
  console.log('[server] Shutting down...');
  clearInterval(meterTimer);
  wss.close();
  server.close();
}

process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);

const PORT = process.env.PORT || 3001;
server.listen(PORT, () => {
  console.log(`[server] moonlitt server listening on http://localhost:${PORT}`);
});
