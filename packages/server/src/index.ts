import express from 'express';
import { createServer } from 'http';
import { WebSocketServer } from 'ws';

const app = express();
const server = createServer(app);
const wss = new WebSocketServer({ server });

wss.on('connection', (ws) => {
  console.log('Client connected');
  ws.send(JSON.stringify({ type: 'state.init', tracks: [], bpm: 120, playing: false }));
  ws.on('message', (data) => {
    console.log('Received:', data.toString());
  });
});

const PORT = process.env.PORT || 3001;
server.listen(PORT, () => {
  console.log(`moonlitt server listening on http://localhost:${PORT}`);
});
