import express from 'express';
import { createServer } from 'http';
import { WebSocketServer, WebSocket } from 'ws';
import multer from 'multer';
import os from 'os';
import { EngineManager } from './engine.js';
import { handleCommand } from './protocol.js';
import type { Command, ServerEvent } from '@moonlitt/protocol';

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

// CORS for the Vite dev server
app.use((_req, res, next) => {
  res.header('Access-Control-Allow-Origin', '*');
  res.header('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
  res.header('Access-Control-Allow-Headers', 'Content-Type');
  next();
});

// --- MIDI file upload endpoint -------------------------------------------

const upload = multer({ dest: os.tmpdir() });

app.post('/api/upload-midi', upload.single('file'), (req, res) => {
  if (!req.file) {
    res.status(400).json({ error: 'No file uploaded' });
    return;
  }

  const trackId = parseInt(req.body.trackId || '0', 10);
  const clip = engine.loadMidi(trackId, req.file.path, req.file.originalname);

  if (!clip) {
    res.status(400).json({ error: `Track ${trackId} not found` });
    return;
  }

  // Broadcast clip addition to all WebSocket clients
  broadcast({ type: 'midi.clip_added', trackId, clip });

  res.json({ ok: true, clip });
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

  // Send current state snapshot
  const state = engine.getState();
  sendEvent(ws, {
    type: 'state.init',
    tracks: state.tracks,
    bpm: state.bpm,
    playing: state.playing,
  });

  ws.on('message', (data) => {
    // Binary frames are not expected from clients; ignore them.
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

    if (response) {
      // Events that affect shared state are broadcast to all clients.
      // Error responses are sent only to the originating client.
      if (response.type === 'error') {
        sendEvent(ws, response);
      } else {
        broadcast(response);
      }
    }
  });

  ws.on('close', () => {
    console.log('[server] Client disconnected');
  });
});

// ---------------------------------------------------------------------------
// 60fps meter broadcast (binary Float32Array)
// ---------------------------------------------------------------------------
// Layout: [trackCount, track0_peakL, track0_peakR, ..., master_peakL, master_peakR]

const METER_INTERVAL_MS = 16; // ~60fps

const meterTimer = setInterval(() => {
  if (!engine.isInitialized()) return;

  // Only send if at least one client is connected
  let hasClient = false;
  for (const ws of wss.clients) {
    if (ws.readyState === WebSocket.OPEN) {
      hasClient = true;
      break;
    }
  }
  if (!hasClient) return;

  const count = engine.trackCount();
  // Float32Array: [count, L0, R0, L1, R1, ..., masterL, masterR]
  const buffer = new Float32Array(1 + count * 2 + 2);
  buffer[0] = count;

  const state = engine.getState();
  for (let i = 0; i < count; i++) {
    const trackId = state.tracks[i]?.id ?? i;
    const levels = engine.getTrackLevels(trackId);
    buffer[1 + i * 2] = levels.peakL;
    buffer[1 + i * 2 + 1] = levels.peakR;
  }

  const master = engine.getMasterLevels();
  buffer[1 + count * 2] = master.peakL;
  buffer[1 + count * 2 + 1] = master.peakR;

  const bytes = Buffer.from(buffer.buffer);
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
  engine.shutdown();
  wss.close();
  server.close();
}

process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);

// ---------------------------------------------------------------------------
// Start
// ---------------------------------------------------------------------------

const PORT = process.env.PORT || 3001;
server.listen(PORT, () => {
  console.log(`[server] moonlitt server listening on http://localhost:${PORT}`);
});
