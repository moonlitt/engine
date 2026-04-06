# moonlitt Web DAW

## Prerequisites
- Node.js 20+
- pnpm 9+
- Rust toolchain (for building moonlitt-node)

## Setup

### 1. Build the native addon
```bash
cd crates/moonlitt-node
npx napi build --release
```

### 2. Install packages
```bash
cd packages
pnpm install
```

### 3. Start the server
```bash
pnpm dev:server
```

### 4. Start the web app (in another terminal)
```bash
pnpm dev
```

### 5. Open http://localhost:5173

## Usage
- Click **"+ Add Track"** in the arrange view to create a track
- Open the **Track Inspector** (right panel) and click **"Load"** to load a .sf2 file
- Play notes with the **virtual keyboard** (bottom) or keys **A-K** on your computer keyboard
- Press **Z/X** to shift octave down/up
- Adjust **volume** and **pan** in the mixer (bottom panel)
- Click the **collapse** button to hide/show the mixer
- Press **Play** or **Space** to start/stop transport
- Edit **BPM** by clicking the tempo display

## Keyboard Shortcuts
| Key | Action |
|-----|--------|
| Space | Play / Stop |
| A-K | Piano keys (C to C+1) |
| Z | Octave down |
| X | Octave up |

## Architecture
```
Browser (React + Zustand) <-> WebSocket <-> Node.js (Express) <-> moonlitt-node (napi-rs) <-> Rust engine <-> cpal <-> soundcard
```
