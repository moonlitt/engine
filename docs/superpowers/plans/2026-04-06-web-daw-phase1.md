# Web DAW Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a functional Web DAW with multi-track mixer, transport, arrange view, virtual keyboard, and track inspector — powered by moonlitt-node over WebSocket.

**Architecture:** React SPA (Vite) communicates with a Node.js server (Express + ws) via WebSocket. The server calls moonlitt-node napi bindings which drive the Rust audio engine. Audio goes directly from Rust to the soundcard. Meters are read from the engine and broadcast to the browser at 60fps.

**Tech Stack:** Vite 6, React 19, Zustand, Tailwind CSS, Express, ws, moonlitt-node (napi-rs), pnpm workspace

**Spec:** `docs/superpowers/specs/2026-04-06-web-daw-phase1-design.md`

**IMPORTANT PRE-REQUISITE:** moonlitt-node must be built before the server can load it. Run `cd crates/moonlitt-node && npm run build` (or `napi build`) to generate the `.node` binary. The server imports this binary.

**KNOWN GAP:** moonlitt-node currently lacks meter reading API (`getTrackLevels()`). Task 1 adds this to the Rust side before building the frontend.

---

## Task 0: Add Meter Reading to moonlitt-node

**Why:** The server needs to read peak levels for all tracks + master at 60fps. Currently moonlitt-node's Session has no meter reading method, and the underlying Runtime doesn't expose it either. The mixer has `LevelMeter` data but it's on the audio thread (inside the processor).

**Approach:** The mixer's meters use atomic floats that can be read from any thread. Add a method to Runtime that reads these atomics, then expose through moonlitt-node.

**Files:**
- Modify: `crates/moonlitt-audio-io/src/runtime.rs` — add `track_meter()` and `master_meter()` methods
- Modify: `crates/moonlitt-node/src/session.rs` — add `getTrackLevels()` napi method
- Modify: `crates/moonlitt-node/src/types.rs` — add `TrackLevels` type if not exists

**Note to implementer:** Read `crates/moonlitt-mixer/src/mixer.rs` to find the `LevelMeter` struct and how `track_meter()` / `master_meter()` work. The mixer is owned by the audio thread (processor), but meter data uses atomic reads so it's safe to access from the main thread. The Runtime holds a reference to the mixer's meters via the command channel or shared atomics.

If direct meter reading isn't possible from Runtime (because the mixer is moved into the audio thread), an alternative approach is:
1. Add `SharedMeter` (using `AtomicU32` for f32 bits) to the session/processor
2. The audio thread writes meter values after each render
3. Runtime reads the atomics

This may require changes in moonlitt-session's processor.rs. The implementer should explore the actual architecture and find the cleanest solution.

- [ ] **Step 1:** Read the mixer, runtime, and processor code to understand where meters are accessible
- [ ] **Step 2:** Implement meter reading in runtime (or session/processor with shared atomics)
- [ ] **Step 3:** Add `getTrackLevels()` to moonlitt-node Session
- [ ] **Step 4:** Test: `cargo test --workspace -- --skip pianoteq --skip keyscape`
- [ ] **Step 5:** Build the node addon: `cd crates/moonlitt-node && npx napi build --release`
- [ ] **Step 6:** Commit

```
feat(node): add meter reading API (track + master levels)
```

---

## Task 1: Project Scaffolding

**Files:**
- Create: `packages/package.json` (workspace root)
- Create: `pnpm-workspace.yaml`
- Create: `packages/protocol/package.json`
- Create: `packages/protocol/src/index.ts`
- Create: `packages/server/package.json`
- Create: `packages/server/tsconfig.json`
- Create: `packages/server/src/index.ts`
- Create: `packages/web/package.json`
- Create: `packages/web/vite.config.ts`
- Create: `packages/web/tsconfig.json`
- Create: `packages/web/index.html`
- Create: `packages/web/src/main.tsx`
- Create: `packages/web/src/App.tsx`
- Create: `packages/web/tailwind.config.js`
- Create: `packages/web/src/styles/globals.css`

- [ ] **Step 1: Create pnpm workspace**

`pnpm-workspace.yaml`:
```yaml
packages:
  - 'packages/*'
```

`packages/package.json`:
```json
{
  "private": true,
  "scripts": {
    "dev": "pnpm --filter @moonlitt/web dev",
    "dev:server": "pnpm --filter @moonlitt/server dev",
    "build": "pnpm -r build"
  }
}
```

- [ ] **Step 2: Create protocol package**

`packages/protocol/package.json`:
```json
{
  "name": "@moonlitt/protocol",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "main": "src/index.ts",
  "types": "src/index.ts"
}
```

`packages/protocol/src/index.ts` — shared types:
```typescript
// Client → Server commands
export type Command =
  | { type: 'transport.play' }
  | { type: 'transport.stop' }
  | { type: 'transport.set_bpm'; bpm: number }
  | { type: 'track.add'; instrumentPath?: string }
  | { type: 'track.remove'; trackId: number }
  | { type: 'track.set_volume'; trackId: number; db: number }
  | { type: 'track.set_pan'; trackId: number; pan: number }
  | { type: 'track.set_mute'; trackId: number; muted: boolean }
  | { type: 'track.set_solo'; trackId: number; solo: boolean }
  | { type: 'track.load_instrument'; trackId: number; path: string }
  | { type: 'master.set_volume'; db: number }
  | { type: 'midi.note_on'; channel: number; note: number; velocity: number }
  | { type: 'midi.note_off'; channel: number; note: number }
  | { type: 'midi.load_file'; trackId: number; path: string }
  | { type: 'insert.add'; trackId: number; effectType: string }
  | { type: 'insert.remove'; trackId: number; insertId: number }
  | { type: 'insert.set_param'; trackId: number; insertId: number; paramId: number; value: number };

// Server → Client events
export type ServerEvent =
  | { type: 'state.init'; tracks: TrackState[]; bpm: number; playing: boolean }
  | { type: 'track.added'; trackId: number; name: string; color: string }
  | { type: 'track.removed'; trackId: number }
  | { type: 'transport.state'; playing: boolean; position: number }
  | { type: 'error'; message: string };

export interface TrackState {
  id: number;
  name: string;
  color: string;
  volume: number;
  pan: number;
  muted: boolean;
  solo: boolean;
  instrumentPath: string | null;
  inserts: InsertState[];
}

export interface InsertState {
  id: number;
  name: string;
  bypassed: boolean;
}

// Track colors cycle
export const TRACK_COLORS = [
  '#4fc3f7', '#81c784', '#ffb74d', '#ef5350',
  '#ab47bc', '#26c6da', '#ff7043', '#66bb6a',
];
```

- [ ] **Step 3: Create server package**

`packages/server/package.json`:
```json
{
  "name": "@moonlitt/server",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "tsx watch src/index.ts",
    "build": "tsc"
  },
  "dependencies": {
    "express": "^4.21.0",
    "ws": "^8.18.0",
    "@moonlitt/protocol": "workspace:*"
  },
  "devDependencies": {
    "@types/express": "^5.0.0",
    "@types/ws": "^8.5.0",
    "tsx": "^4.19.0",
    "typescript": "^5.7.0"
  }
}
```

`packages/server/tsconfig.json`:
```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "esModuleInterop": true,
    "outDir": "dist",
    "rootDir": "src"
  },
  "include": ["src"]
}
```

`packages/server/src/index.ts` — minimal server:
```typescript
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
```

- [ ] **Step 4: Create web package**

`packages/web/package.json`:
```json
{
  "name": "@moonlitt/web",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vite build"
  },
  "dependencies": {
    "react": "^19.0.0",
    "react-dom": "^19.0.0",
    "zustand": "^5.0.0",
    "@moonlitt/protocol": "workspace:*"
  },
  "devDependencies": {
    "@types/react": "^19.0.0",
    "@types/react-dom": "^19.0.0",
    "@vitejs/plugin-react": "^4.3.0",
    "autoprefixer": "^10.4.0",
    "postcss": "^8.4.0",
    "tailwindcss": "^3.4.0",
    "typescript": "^5.7.0",
    "vite": "^6.0.0"
  }
}
```

`packages/web/vite.config.ts`:
```typescript
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  server: { port: 5173 },
});
```

`packages/web/index.html`:
```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>moonlitt</title>
</head>
<body class="bg-[#12121e] text-[#e0e0e0] overflow-hidden">
  <div id="root"></div>
  <script type="module" src="/src/main.tsx"></script>
</body>
</html>
```

`packages/web/tailwind.config.js`:
```javascript
export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        'daw-bg': '#12121e',
        'daw-panel': '#1a1a2e',
        'daw-surface': '#16162a',
        'daw-control': '#252540',
        'daw-border': '#2a2a40',
        'daw-accent': '#7c4dff',
      },
    },
  },
  plugins: [],
};
```

`packages/web/src/styles/globals.css`:
```css
@tailwind base;
@tailwind components;
@tailwind utilities;
```

`packages/web/src/main.tsx`:
```tsx
import React from 'react';
import ReactDOM from 'react-dom/client';
import { App } from './App';
import './styles/globals.css';

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
```

`packages/web/src/App.tsx`:
```tsx
export function App() {
  return (
    <div className="h-screen flex flex-col bg-daw-bg text-[#e0e0e0] font-sans text-sm">
      {/* Transport Bar */}
      <div className="h-12 bg-daw-panel border-b border-daw-border flex items-center px-4">
        <span className="text-daw-accent font-bold">moonlitt</span>
      </div>

      {/* Main Area */}
      <div className="flex-1 flex overflow-hidden">
        <div className="flex-1 bg-daw-surface">
          {/* Arrange View placeholder */}
          <div className="flex items-center justify-center h-full text-[#555]">
            Arrange View
          </div>
        </div>
        <div className="w-[220px] bg-daw-panel border-l border-daw-border p-3">
          {/* Track Inspector placeholder */}
          <div className="text-[#555] text-xs">Track Inspector</div>
        </div>
      </div>

      {/* Mixer */}
      <div className="h-40 bg-daw-panel border-t-2 border-daw-border p-3">
        <div className="text-[#555] text-xs">Mixer</div>
      </div>

      {/* Virtual Keyboard */}
      <div className="h-16 bg-daw-surface border-t border-daw-border p-2">
        <div className="text-[#555] text-xs">Virtual Keyboard</div>
      </div>
    </div>
  );
}
```

- [ ] **Step 5: Install dependencies and verify**

```bash
cd packages && pnpm install
pnpm --filter @moonlitt/web dev  # Should open on localhost:5173
```

Verify: browser shows the 5-area layout skeleton with placeholder text.

- [ ] **Step 6: Commit**

```bash
git add packages/ pnpm-workspace.yaml
git commit -m "feat(daw): scaffold pnpm workspace with web + server + protocol

Vite + React + Tailwind (web), Express + WebSocket (server),
shared types (protocol). 5-area layout skeleton."
```

---

## Task 2: Zustand Stores + WebSocket Connection

**Files:**
- Create: `packages/web/src/stores/session.ts`
- Create: `packages/web/src/stores/transport.ts`
- Create: `packages/web/src/stores/mixer.ts`
- Create: `packages/web/src/hooks/useWebSocket.ts`

The implementer should create the three Zustand stores and the WebSocket hook as defined in the spec. Key points:

- `sessionStore`: manages WebSocket connection, `send(command)` method
- `transportStore`: playing, bpm, position — updated from server events
- `mixerStore`: tracks array, selectedTrackId, masterVolume, meter data — `updateMeters(data)` is called at 60fps from binary WebSocket frames
- `useWebSocket`: React hook that connects on mount, dispatches incoming events to stores, handles binary meter frames separately from JSON events

The `send()` method on sessionStore sends JSON-stringified Command objects. Binary frames from server are meter data (Float32Array).

- [ ] **Step 1:** Create all store files with the types from the spec
- [ ] **Step 2:** Create useWebSocket hook
- [ ] **Step 3:** Wire into App.tsx (call useWebSocket on mount)
- [ ] **Step 4:** Verify: dev server connects to ws://localhost:3001, shows "Client connected" in server console
- [ ] **Step 5:** Commit

```
feat(daw): add Zustand stores and WebSocket connection

Transport, mixer, and session stores.
WebSocket hook with binary meter frame support.
```

---

## Task 3: Server Engine Integration

**Files:**
- Create: `packages/server/src/engine.ts`
- Create: `packages/server/src/protocol.ts`
- Modify: `packages/server/src/index.ts`

The implementer should:

1. Create `engine.ts` that wraps moonlitt-node. Import the native addon from the built `.node` file. The path will be something like `../../crates/moonlitt-node/moonlitt.darwin-arm64.node` (platform-specific). Use a try/catch to handle the case where the addon isn't built yet.

2. Create `protocol.ts` that maps incoming WebSocket commands to engine method calls.

3. Update `index.ts` to:
   - Initialize the engine on startup
   - Handle WebSocket commands via protocol.ts
   - Start a 60fps meter broadcast timer
   - Serve static files from web build (for production)

Key: the moonlitt-node addon must be built first (`cd crates/moonlitt-node && npx napi build`). The server should gracefully degrade if the addon isn't available (log a warning, serve UI without audio).

- [ ] **Step 1:** Create engine.ts wrapper
- [ ] **Step 2:** Create protocol.ts command handler
- [ ] **Step 3:** Update index.ts with engine init + command routing + meter broadcast
- [ ] **Step 4:** Test: start server, connect from browser, send play command, verify no crash
- [ ] **Step 5:** Commit

```
feat(daw): integrate moonlitt-node engine with WebSocket server

Engine wrapper, command protocol, 60fps meter broadcast.
```

---

## Task 4: TransportBar Component

**Files:**
- Create: `packages/web/src/components/TransportBar.tsx`
- Modify: `packages/web/src/App.tsx`

Implement the transport bar with: Play/Stop toggle, BPM editor (click to edit), position display (bar:beat:tick), time signature display. All controls send WebSocket commands via sessionStore.

- [ ] **Step 1:** Create TransportBar.tsx
- [ ] **Step 2:** Wire into App.tsx (replace placeholder)
- [ ] **Step 3:** Verify: clicking Play sends `transport.play`, BPM edit sends `transport.set_bpm`
- [ ] **Step 4:** Commit

```
feat(daw): add TransportBar component

Play/Stop, BPM editor, position display, time signature.
```

---

## Task 5: Mixer Components (ChannelStrip, Fader, Meter, PanKnob)

**Files:**
- Create: `packages/web/src/components/Mixer.tsx`
- Create: `packages/web/src/components/ChannelStrip.tsx`
- Create: `packages/web/src/components/Fader.tsx`
- Create: `packages/web/src/components/PanKnob.tsx`
- Create: `packages/web/src/components/Meter.tsx`
- Modify: `packages/web/src/App.tsx`

This is the largest UI task. Key details:

- **Fader**: vertical slider, range -∞ to +6dB, drag interaction, shows dB value
- **PanKnob**: circular knob or horizontal slider, range -1 to +1
- **Meter**: dual vertical bars (L/R), green/yellow/red gradient, peak hold indicator (falls after 2s), updates at 60fps from mixerStore
- **ChannelStrip**: combines Fader + PanKnob + Meter + Mute/Solo buttons + track name
- **Mixer**: horizontal row of ChannelStrips + Master strip, collapsible

Meter rendering should use `requestAnimationFrame` for smooth 60fps updates, NOT React re-renders. Use a canvas or refs to update meter bars directly.

- [ ] **Step 1:** Create Meter.tsx (canvas-based for performance)
- [ ] **Step 2:** Create Fader.tsx (drag interaction)
- [ ] **Step 3:** Create PanKnob.tsx
- [ ] **Step 4:** Create ChannelStrip.tsx (compose above)
- [ ] **Step 5:** Create Mixer.tsx (row of strips + master + collapse toggle)
- [ ] **Step 6:** Wire into App.tsx
- [ ] **Step 7:** Verify: fader drag sends set_volume, meters animate
- [ ] **Step 8:** Commit

```
feat(daw): add Mixer with ChannelStrip, Fader, PanKnob, Meter

60fps canvas-based meters, drag faders, pan knobs.
Mute/Solo buttons, master strip, collapsible panel.
```

---

## Task 6: ArrangeView (Timeline + Track Headers)

**Files:**
- Create: `packages/web/src/components/ArrangeView.tsx`
- Create: `packages/web/src/components/TrackHeader.tsx`
- Create: `packages/web/src/components/MidiClip.tsx`
- Create: `packages/web/src/components/Playhead.tsx`
- Create: `packages/web/src/components/TimelineRuler.tsx`
- Modify: `packages/web/src/App.tsx`

- ArrangeView: scrollable area with track lanes
- TrackHeader: left column (120px), shows name, color bar, mute/solo
- MidiClip: colored rectangle on timeline (position + length in bars)
- Playhead: vertical line at current transport position, animated during playback
- TimelineRuler: bar numbers at top
- Add Track button at bottom

Horizontal scrolling via CSS overflow. Playhead position calculated from transportStore.position.

- [ ] **Step 1-6:** Create each component, wire into App.tsx
- [ ] **Step 7:** Commit

```
feat(daw): add ArrangeView with timeline, tracks, clips, playhead

Track headers, MIDI clip display, animated playhead,
timeline ruler, horizontal scrolling, Add Track button.
```

---

## Task 7: VirtualKeyboard

**Files:**
- Create: `packages/web/src/components/VirtualKeyboard.tsx`
- Create: `packages/web/src/hooks/useKeyboard.ts`
- Modify: `packages/web/src/App.tsx`

- 2-octave piano keyboard (C3-C5), rendered as CSS divs (white + black keys)
- Mouse: mousedown = note_on, mouseup = note_off
- Computer keyboard mapping: A=C3, W=C#3, S=D3, E=D#3, D=E3, F=F3, T=F#3, G=G3, Y=G#3, H=A3, U=A#3, J=B3, K=C4
- Z/X = octave down/up
- Notes sent to selected track via sessionStore
- useKeyboard hook handles keydown/keyup events, prevents key repeat

- [ ] **Step 1:** Create VirtualKeyboard.tsx
- [ ] **Step 2:** Create useKeyboard.ts hook
- [ ] **Step 3:** Wire into App.tsx
- [ ] **Step 4:** Verify: pressing 'A' key sends note_on(0, 60, 100), releasing sends note_off
- [ ] **Step 5:** Commit

```
feat(daw): add VirtualKeyboard with mouse + computer key input

2-octave display, ASDFGHJK mapping, octave shift Z/X.
Notes sent to selected track.
```

---

## Task 8: TrackInspector

**Files:**
- Create: `packages/web/src/components/TrackInspector.tsx`
- Modify: `packages/web/src/App.tsx`

Right panel showing details of the selected track:
- Instrument section: .sf2 file name, program name, "Change" button
- Insert chain: ordered list of effects with bypass indicators
- "Add Insert" button (dropdown with effect types: EQ, Compressor, Reverb, Delay, Chorus, etc.)

Sends `insert.add` and `track.load_instrument` commands.

- [ ] **Step 1:** Create TrackInspector.tsx
- [ ] **Step 2:** Wire into App.tsx
- [ ] **Step 3:** Commit

```
feat(daw): add TrackInspector (instrument + insert chain)

Shows selected track details, instrument info,
insert chain with bypass, add insert dropdown.
```

---

## Task 9: InstrumentSelector (File Dialog)

**Files:**
- Create: `packages/web/src/components/InstrumentSelector.tsx`
- Modify: `packages/server/src/protocol.ts` — add file browse handler

Modal dialog for selecting .sf2 files:
- Server-side: list .sf2 files in common directories (or accept a path)
- Client-side: modal with file list, click to select, "Load" button
- Alternative: simple text input for file path (simpler, works cross-platform)

For Phase 1 MVP, a text input for the file path is sufficient. Full file browser can come later.

- [ ] **Step 1:** Create InstrumentSelector.tsx (modal with path input)
- [ ] **Step 2:** Wire to TrackInspector's "Change" button
- [ ] **Step 3:** Test: enter .sf2 path, click Load, verify sound plays
- [ ] **Step 4:** Commit

```
feat(daw): add InstrumentSelector (file path input modal)

Enter .sf2 path to load instruments into tracks.
```

---

## Task 10: MIDI File Loading

**Files:**
- Modify: `packages/server/src/engine.ts` — add MIDI file loading
- Modify: `packages/server/src/protocol.ts` — handle midi.load_file command
- Modify: `packages/web/src/components/ArrangeView.tsx` — file drop zone

Support loading MIDI files onto tracks:
- Drag & drop .mid file onto a track lane in ArrangeView
- Or use a file dialog from the track header
- Server loads the MIDI file into the sequencer via moonlitt-node
- Client displays MIDI clips on the timeline (colored blocks)

Note: moonlitt-node's Session needs a `loadMidiFile()` method. Check if the Runtime has sequencer support (`load_midi` or similar). If not, add it.

- [ ] **Step 1:** Add MIDI loading to server engine
- [ ] **Step 2:** Add drag-drop to ArrangeView
- [ ] **Step 3:** Display loaded clips on timeline
- [ ] **Step 4:** Test: drag .mid file, press Play, hear audio
- [ ] **Step 5:** Commit

```
feat(daw): add MIDI file loading (drag & drop onto tracks)

Load .mid files into sequencer, display clips on timeline.
```

---

## Task 11: Polish + Panel Resizing

**Files:**
- Various component modifications for:
  - Mixer panel resize (drag border to resize height)
  - Right panel collapse toggle
  - Keyboard shortcuts (Space = play/stop, etc.)
  - Error toast notifications
  - Loading state indicators
  - Track color cycling on add
  - Responsive layout adjustments

- [ ] **Step 1:** Add panel resize (CSS resize or drag handle)
- [ ] **Step 2:** Add keyboard shortcuts (Space, Ctrl+S placeholder)
- [ ] **Step 3:** Add error handling (toast notifications)
- [ ] **Step 4:** Polish visual details (hover states, transitions, focus rings)
- [ ] **Step 5:** Commit

```
feat(daw): polish — panel resize, keyboard shortcuts, error handling

Draggable mixer height, Space=play/stop, error toasts.
```

---

## Task 12: Integration Test + README

**Files:**
- Create: `packages/web/src/__tests__/smoke.test.ts` (optional, basic render test)
- Create: `packages/README.md` — how to run the DAW

README should cover:
```markdown
# moonlitt Web DAW

## Prerequisites
- Node.js 20+
- pnpm 9+
- Rust toolchain (for building moonlitt-node)

## Setup
1. Build the native addon: `cd crates/moonlitt-node && npx napi build --release`
2. Install packages: `cd packages && pnpm install`
3. Start the server: `pnpm dev:server`
4. Start the web app: `pnpm dev`
5. Open http://localhost:5173

## Usage
- Click "Add Track" to create a track
- Load a .sf2 file via the Track Inspector
- Play notes with the virtual keyboard (or keys A-K)
- Adjust volume/pan in the mixer
- Drag a .mid file onto a track to load MIDI
- Press Space to play/stop
```

- [ ] **Step 1:** Create README
- [ ] **Step 2:** Manual end-to-end test: load SF2, play keyboard, verify audio
- [ ] **Step 3:** Commit

```
docs: add Web DAW README with setup instructions
```
