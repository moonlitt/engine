# Web DAW Phase 1 Design Spec — Mixer + Transport

**Date:** 2026-04-06
**Status:** Draft
**Scope:** Phase 1 of Web DAW — Transport, Arrange view, Mixer, Virtual Keyboard, Track Inspector

## Motivation

The moonlitt engine has 20 effects, 14 crates, 365 tests, SIMD optimization, external sidechain routing, and oversampling. A Web DAW provides the most direct way to verify and demonstrate these capabilities visually and audibly. Phase 1 delivers a functional mixing + playback workstation.

## Architecture

```
Browser (Vite + React + Zustand)
    ↕ WebSocket (JSON commands + binary meter data)
Node.js Server (Express + ws)
    ↕ napi-rs (direct function calls, no serialization)
moonlitt-node (Rust native addon)
    ↕
moonlitt engine (audio-io → cpal → soundcard)
```

- **Browser** sends commands (play, stop, note_on, set_volume, load_sf2, add_track...)
- **Server** translates to moonlitt-node API calls
- **Meter data** flows server→browser at 60fps via binary WebSocket frames
- **Audio** goes directly from Rust engine to soundcard (never passes through browser)

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Build tool | Vite 6 |
| UI framework | React 19 |
| State management | Zustand |
| Styling | Tailwind CSS |
| WebSocket | ws (server), native WebSocket (client) |
| HTTP server | Express |
| Native addon | moonlitt-node (napi-rs) |
| Package manager | pnpm |

## Directory Structure

```
packages/
├── server/                    Node.js backend
│   ├── package.json
│   ├── src/
│   │   ├── index.ts           Express + WebSocket server
│   │   ├── engine.ts          moonlitt-node wrapper (Session lifecycle)
│   │   └── protocol.ts        WebSocket message types
│   └── tsconfig.json
│
├── web/                       React frontend
│   ├── package.json
│   ├── vite.config.ts
│   ├── index.html
│   ├── src/
│   │   ├── main.tsx           Entry point
│   │   ├── App.tsx            Root layout (5 areas)
│   │   ├── stores/
│   │   │   ├── transport.ts   Transport state (playing, bpm, position)
│   │   │   ├── mixer.ts       Tracks, volumes, pans, meters, mute/solo
│   │   │   └── session.ts     WebSocket connection, engine commands
│   │   ├── components/
│   │   │   ├── TransportBar.tsx
│   │   │   ├── ArrangeView.tsx
│   │   │   ├── TrackHeader.tsx
│   │   │   ├── MidiClip.tsx
│   │   │   ├── Playhead.tsx
│   │   │   ├── Mixer.tsx
│   │   │   ├── ChannelStrip.tsx
│   │   │   ├── Fader.tsx
│   │   │   ├── PanKnob.tsx
│   │   │   ├── Meter.tsx
│   │   │   ├── VirtualKeyboard.tsx
│   │   │   ├── TrackInspector.tsx
│   │   │   └── InstrumentSelector.tsx
│   │   ├── hooks/
│   │   │   ├── useWebSocket.ts
│   │   │   └── useKeyboard.ts
│   │   └── styles/
│   │       └── globals.css
│   └── tsconfig.json
│
├── protocol/                  Shared types (server + web)
│   ├── package.json
│   └── src/
│       └── index.ts           Command/Event types, message format
│
└── package.json               pnpm workspace root
```

Plus root `pnpm-workspace.yaml`:
```yaml
packages:
  - 'packages/*'
```

## WebSocket Protocol

### Client → Server (Commands)

```typescript
type Command =
  | { type: 'transport.play' }
  | { type: 'transport.stop' }
  | { type: 'transport.set_bpm', bpm: number }
  | { type: 'track.add', instrumentPath?: string }
  | { type: 'track.remove', trackId: number }
  | { type: 'track.set_volume', trackId: number, db: number }
  | { type: 'track.set_pan', trackId: number, pan: number }
  | { type: 'track.set_mute', trackId: number, muted: boolean }
  | { type: 'track.set_solo', trackId: number, solo: boolean }
  | { type: 'track.load_instrument', trackId: number, path: string }
  | { type: 'master.set_volume', db: number }
  | { type: 'midi.note_on', channel: number, note: number, velocity: number }
  | { type: 'midi.note_off', channel: number, note: number }
  | { type: 'midi.load_file', trackId: number, path: string }
  | { type: 'insert.add', trackId: number, effectType: string }
  | { type: 'insert.remove', trackId: number, insertId: number }
  | { type: 'insert.set_param', trackId: number, insertId: number, paramId: number, value: number }
```

### Server → Client (Events)

```typescript
type Event =
  | { type: 'state.init', tracks: TrackState[], bpm: number, playing: boolean }
  | { type: 'track.added', trackId: number, name: string, color: string }
  | { type: 'track.removed', trackId: number }
  | { type: 'transport.state', playing: boolean, position: number }
  | { type: 'meters', data: Float32Array }  // binary frame: [trackId, peakL, peakR, ...] × N tracks + master
  | { type: 'error', message: string }
```

### Meter Data (Binary)

Meters update at 60fps. Binary format for efficiency:
```
[track_count: u8] [track0_peak_l: f32] [track0_peak_r: f32] [track1_peak_l: f32] ... [master_peak_l: f32] [master_peak_r: f32]
```

Server runs a 16ms timer, reads levels from moonlitt-node, packs into binary, sends to all connected clients.

---

## UI Components

### TransportBar (top, fixed height 48px)

```
[moonlitt logo] [◀◀] [■ Stop] [▶ Play] [⏺ Rec*] | POS 001:1:000 | BPM 120.0 | SIG 4/4
```

- Play/Stop toggle controls `transport.play` / `transport.stop`
- BPM is editable (click to edit, Enter to confirm)
- Position updates from server at 60fps
- Record button is visible but disabled (gray, Phase 3)

### ArrangeView (main area, scrollable)

- **Track headers** (left column, 120px wide): track name, color bar, mute/solo buttons
- **Timeline** (right of headers, horizontally scrollable): MIDI clips as colored rectangles
- **Playhead**: vertical purple line at current position, moves during playback
- **Timeline ruler**: bar numbers at top
- **Add Track button** at bottom of track list
- Track selection: click track header to select (highlights, shows in Inspector)
- Drag MIDI file onto track to load (or use file dialog)

### Mixer (bottom, collapsible, default height 160px)

- **Channel strips**: one per track + master
- Each strip: Mute/Solo buttons, Pan knob, Stereo meter, Fader (vertical), Track name
- **Master strip**: wider, separated by divider, stereo meter with peak hold
- Fader range: -inf to +6dB, default 0dB
- Pan range: -1 (L) to +1 (R), default center
- Meter: green 0-70%, yellow 70-90%, red >90%. Peak hold (falls after 2s)
- Collapse/expand via button or drag handle

### VirtualKeyboard (bottom panel, togglable, 60px height)

- 2-octave piano keyboard (C3-C5 default, scrollable)
- Mouse click to play note_on/note_off
- Computer keyboard mapping: A=C, W=C#, S=D, E=D#, D=E, F=F, T=F#, G=G, Y=G#, H=A, U=A#, J=B, K=C+1
- Octave shift: Z (down), X (up)
- Velocity: fixed at 100 (or adjustable slider)
- Notes sent to currently selected track

### TrackInspector (right panel, collapsible, 220px wide)

- Shows details for the selected track
- **Instrument section**: loaded .sf2 file name, program name, "Change" button
- **Insert Chain section**: ordered list of insert effects, each with name + bypass indicator
- "Add Insert" button opens effect selector dropdown
- Click insert name to expand inline parameter editor (Phase 2 will have a proper panel)

### InstrumentSelector (modal dialog)

- File browser for .sf2 files
- Shows file path + file size
- "Load" button to confirm
- Triggered by "Change" in Inspector or when adding a new track

---

## State Management (Zustand)

### transportStore

```typescript
interface TransportStore {
  playing: boolean;
  bpm: number;
  position: number;         // in ticks or samples
  timeSignature: [number, number];

  play(): void;
  stop(): void;
  setBpm(bpm: number): void;
  updatePosition(pos: number): void;
}
```

### mixerStore

```typescript
interface Track {
  id: number;
  name: string;
  color: string;
  volume: number;           // dB
  pan: number;              // -1..1
  muted: boolean;
  solo: boolean;
  peakL: number;            // 0..1, updated at 60fps
  peakR: number;
  instrumentPath: string | null;
  instrumentName: string | null;
  inserts: InsertEffect[];
  clips: MidiClip[];
}

interface InsertEffect {
  id: number;
  name: string;
  bypassed: boolean;
}

interface MidiClip {
  id: number;
  name: string;
  startBar: number;
  lengthBars: number;
}

interface MixerStore {
  tracks: Track[];
  selectedTrackId: number | null;
  masterVolume: number;
  masterPeakL: number;
  masterPeakR: number;

  addTrack(name?: string): void;
  removeTrack(id: number): void;
  setVolume(id: number, db: number): void;
  setPan(id: number, pan: number): void;
  setMute(id: number, muted: boolean): void;
  setSolo(id: number, solo: boolean): void;
  selectTrack(id: number): void;
  updateMeters(data: Float32Array): void;
  loadInstrument(trackId: number, path: string): void;
  addInsert(trackId: number, effectType: string): void;
}
```

### sessionStore

```typescript
interface SessionStore {
  connected: boolean;
  ws: WebSocket | null;

  connect(url: string): void;
  send(command: Command): void;
  disconnect(): void;
}
```

---

## Server (Node.js)

### engine.ts — moonlitt-node wrapper

```typescript
import * as moonlitt from '@moonlitt/node';

class EngineManager {
  private session: moonlitt.Session | null = null;

  init(sampleRate: number, bufferSize: number): void {
    // Create session via moonlitt-node
  }

  addTrack(instrumentPath?: string): number {
    // moonlitt.create(path) → session.addTrack(backend)
    return trackId;
  }

  play(): void { this.session?.play(); }
  stop(): void { this.session?.stop(); }
  noteOn(ch: number, note: number, vel: number): void { this.session?.noteOn(ch, note, vel); }

  getTrackLevels(): Float32Array {
    // Read meter levels for all tracks + master
  }
}
```

### index.ts — Server entry

```typescript
const app = express();
const server = http.createServer(app);
const wss = new WebSocketServer({ server });

const engine = new EngineManager();
engine.init(44100, 512);

// WebSocket handler
wss.on('connection', (ws) => {
  // Send initial state
  ws.send(JSON.stringify({ type: 'state.init', ... }));

  ws.on('message', (data) => {
    const cmd = JSON.parse(data.toString());
    handleCommand(engine, cmd);
  });
});

// Meter broadcast at 60fps
setInterval(() => {
  const meters = engine.getTrackLevels();
  wss.clients.forEach(ws => ws.send(meters));  // binary
}, 16);

server.listen(3000);
```

---

## Color Theme

Dark theme matching the mockup:

| Token | Value | Usage |
|-------|-------|-------|
| bg-primary | #12121e | Main background |
| bg-secondary | #1a1a2e | Panels, track headers |
| bg-tertiary | #252540 | Buttons, inputs |
| bg-surface | #16162a | Track lanes |
| border | #2a2a40 | Dividers |
| text-primary | #e0e0e0 | Main text |
| text-secondary | #888888 | Labels |
| text-muted | #555555 | Disabled |
| accent | #7c4dff | Brand purple, playhead |
| meter-green | #4caf50 | Meter safe |
| meter-yellow | #ffeb3b | Meter warning |
| meter-red | #f44336 | Meter clip |

Track colors cycle through: #4fc3f7, #81c784, #ffb74d, #ef5350, #ab47bc, #26c6da, #ff7043, #66bb6a

---

## Implementation Order

```
(1) Project scaffolding: pnpm workspace, Vite, React, Tailwind, Express
(2) WebSocket protocol: shared types, connection management
(3) Server: engine.ts wrapper around moonlitt-node
(4) TransportBar component + transport store
(5) Mixer component (ChannelStrip, Fader, Meter, PanKnob)
(6) Meter data pipeline (server→client binary WebSocket at 60fps)
(7) ArrangeView (TrackHeader, timeline, playhead, MIDI clips)
(8) VirtualKeyboard (mouse + computer keyboard)
(9) TrackInspector (instrument info, insert chain preview)
(10) InstrumentSelector (file dialog, .sf2 loading)
(11) MIDI file loading (drag onto track or file dialog)
(12) Polish: resize panels, keyboard shortcuts, error handling
```

---

## Testing Strategy

- **Component tests**: React Testing Library for UI components
- **Integration tests**: WebSocket round-trip (send command, verify state change)
- **E2E tests**: Playwright (Phase 2+, not Phase 1 MVP)
- **Manual testing**: Load SF2, play keyboard, verify audio output through speakers

## Out of Scope (Phase 2+)

- Effect parameter editing panel (full GUI per effect)
- Send/return routing UI
- Piano Roll / MIDI editor
- Audio recording / waveform display
- Session save/load
- Plugin scanning (VST3/CLAP)
- Group track / sidechain routing UI
- Undo/redo
