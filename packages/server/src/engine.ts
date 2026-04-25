/**
 * Engine wrapper for moonlitt-node native addon.
 *
 * Provides a high-level API for the WebSocket server to drive the audio engine.
 * Gracefully degrades to no-op mock mode when the native addon is not built.
 */

// TrackState and TrackMeta share the same shape; we define TrackMeta locally
// to avoid coupling the engine internals to the protocol's exact type.

// ---------------------------------------------------------------------------
// Native addon import
// ---------------------------------------------------------------------------

interface NativeTrackLevels {
  peakL: number;
  peakR: number;
}

interface NativeParamInfo {
  id: number;
  name: string;
  group: string;
  min: number;
  max: number;
  default: number;
  stepCount: number;
}

interface NativeBackend {
  sampleRate(): number;
  paramCount(): number;
  setParam(id: number, value: number): void;
  getParam(id: number): number | null;
  paramInfo(index: number): NativeParamInfo | null;
  paramDisplay(id: number, value: number): string | null;
  isConsumed(): boolean;
}

interface NativeSession {
  start(): void;
  stop(): void;
  play(): void;
  pause(): void;
  stopPlayback(): void;
  isPlaying(): boolean;
  setTempo(bpm: number): void;
  setLoop(enabled: boolean): void;
  loadMidi(path: string): void;
  unloadMidi(): void;
  swapTrackBackend(trackId: number, backend: NativeBackend): void;
  noteOn(channel: number, note: number, velocity: number): void;
  noteOff(channel: number, note: number): void;
  cc(channel: number, cc: number, value: number): void;
  pitchBend(channel: number, value: number): void;
  programChange(channel: number, program: number): void;
  allNotesOff(): void;
  setVolume(volume: number): void;
  setParam(id: number, value: number): void;
  addTrack(backend: NativeBackend, channelMask: number): number;
  removeTrack(trackId: number): void;
  addInsert(trackId: number, effect: NativeBackend): number;
  removeInsert(trackId: number, insertId: number): void;
  addSendBus(effect: NativeBackend): number;
  setTrackVolume(trackId: number, volume: number): void;
  setTrackPan(trackId: number, pan: number): void;
  setTrackTrim(trackId: number, trimDb: number): void;
  setTrackMute(trackId: number, mute: boolean): void;
  setTrackSolo(trackId: number, solo: boolean): void;
  setTrackSend(trackId: number, busId: number, level: number): void;
  setMasterVolume(volume: number): void;
  setInsertBypass(trackId: number, insertId: number, bypass: boolean): void;
  setTrackRoute(trackId: number, targetId: number): void;
  setParamForTrack(trackId: number, paramId: number, value: number): void;
  setInsertParam(trackId: number, insertId: number, paramId: number, value: number): void;
  setSendBusParam(busId: number, paramId: number, value: number): void;
  trackLevels(trackId: number): NativeTrackLevels;
  masterLevels(): NativeTrackLevels;
  trackCount(): number;
  droppedEvents(): number;
  shutdown(): void;
}

interface NativePluginInfo {
  name: string;
  path: string;
  format: string;
}

interface NativeAddon {
  create(path: string, sampleRate: number, bufferSize: number): NativeBackend;
  scanPlugins(sampleRate: number, bufferSize: number): Array<NativePluginInfo>;
  supportedFormats(): string[];
  createEq(sampleRate: number): NativeBackend;
  createCompressor(sampleRate: number): NativeBackend;
  createReverb(sampleRate: number): NativeBackend;
  createDattorroReverb(sampleRate: number): NativeBackend;
  createLimiter(sampleRate: number): NativeBackend;
  createGate(sampleRate: number): NativeBackend;
  createDeesser(sampleRate: number): NativeBackend;
  createStereoDelay(sampleRate: number): NativeBackend;
  createChorus(sampleRate: number): NativeBackend;
  createFlanger(sampleRate: number): NativeBackend;
  createPhaser(sampleRate: number): NativeBackend;
  createTremolo(sampleRate: number): NativeBackend;
  createSaturator(sampleRate: number): NativeBackend;
  createBitcrusher(sampleRate: number): NativeBackend;
  createMultibandCompressor(sampleRate: number): NativeBackend;
  createAutoFilter(sampleRate: number): NativeBackend;
  createPitchShifter(sampleRate: number): NativeBackend;
  createGain(sampleRate: number): NativeBackend;
  createStereoWidth(sampleRate: number): NativeBackend;
  Session: {
    create(backend: NativeBackend, sampleRate: number, bufferSize: number): NativeSession;
  };
}

let addon: NativeAddon | null = null;

try {
  // napi-rs build output -- platform-specific .node binary with JS wrapper.
  // After `cd crates/moonlitt-node && npx napi build`, this produces index.js + .node file.
  // index.js is CommonJS doing `module.exports = require('./moonlitt.node')`, so under ESM
  // dynamic import the real addon lives on `.default` rather than on the top-level binding.
  const imported = await import('../../../crates/moonlitt-node/index.js');
  const candidate = (imported as { default?: unknown }).default ?? imported;
  addon = candidate as NativeAddon;
  if (typeof (addon as { createGain?: unknown }).createGain !== 'function') {
    console.error('[engine] addon loaded but does not look right:', Object.keys(addon as object));
    addon = null;
  }
} catch (e) {
  console.warn('[engine] moonlitt-node addon not found:', (e as Error).message);
  console.warn('[engine] Build it with: cd crates/moonlitt-node && npx napi build');
}

// ---------------------------------------------------------------------------
// Track color cycling
// ---------------------------------------------------------------------------

const COLORS = [
  '#4fc3f7', '#81c784', '#ffb74d', '#ef5350',
  '#ab47bc', '#26c6da', '#ff7043', '#66bb6a',
];

// ---------------------------------------------------------------------------
// Effect factory lookup
// ---------------------------------------------------------------------------

const EFFECT_FACTORIES: Record<string, (addon: NativeAddon, sr: number) => NativeBackend> = {
  eq:                    (a, sr) => a.createEq(sr),
  compressor:            (a, sr) => a.createCompressor(sr),
  reverb:                (a, sr) => a.createReverb(sr),
  'dattorro-reverb':     (a, sr) => a.createDattorroReverb(sr),
  limiter:               (a, sr) => a.createLimiter(sr),
  gate:                  (a, sr) => a.createGate(sr),
  deesser:               (a, sr) => a.createDeesser(sr),
  'stereo-delay':        (a, sr) => a.createStereoDelay(sr),
  chorus:                (a, sr) => a.createChorus(sr),
  flanger:               (a, sr) => a.createFlanger(sr),
  phaser:                (a, sr) => a.createPhaser(sr),
  tremolo:               (a, sr) => a.createTremolo(sr),
  saturator:             (a, sr) => a.createSaturator(sr),
  bitcrusher:            (a, sr) => a.createBitcrusher(sr),
  'multiband-compressor': (a, sr) => a.createMultibandCompressor(sr),
  'auto-filter':         (a, sr) => a.createAutoFilter(sr),
  'pitch-shifter':       (a, sr) => a.createPitchShifter(sr),
  gain:                  (a, sr) => a.createGain(sr),
  'stereo-width':        (a, sr) => a.createStereoWidth(sr),
};

// ---------------------------------------------------------------------------
// Track metadata (server-side, not in the native addon)
// ---------------------------------------------------------------------------

interface ClipMeta {
  id: number;
  name: string;
  startBar: number;
  lengthBars: number;
}

interface ParamMeta {
  id: number;
  name: string;
  group: string;
  min: number;
  max: number;
  default: number;
  stepCount: number;
  value: number;
}

interface InsertMeta {
  id: number;
  name: string;
  bypassed: boolean;
  params: ParamMeta[];
}

interface TrackMeta {
  id: number;
  name: string;
  color: string;
  volume: number;
  pan: number;
  muted: boolean;
  solo: boolean;
  instrumentPath: string | null;
  inserts: InsertMeta[];
  clips: ClipMeta[];
}

/// Snapshot a backend's full parameter list (called before the backend is
/// consumed by addInsert). Each entry carries its current value.
function snapshotParams(backend: NativeBackend): ParamMeta[] {
  const out: ParamMeta[] = [];
  const count = backend.paramCount();
  for (let i = 0; i < count; i++) {
    const info = backend.paramInfo(i);
    if (!info) continue;
    const value = backend.getParam(info.id);
    out.push({
      id: info.id,
      name: info.name,
      group: info.group,
      min: info.min,
      max: info.max,
      default: info.default,
      stepCount: info.stepCount,
      value: value ?? info.default,
    });
  }
  return out;
}

// ---------------------------------------------------------------------------
// EngineManager
// ---------------------------------------------------------------------------

export class EngineManager {
  private session: NativeSession | null = null;
  private readonly sampleRate: number;
  private readonly bufferSize: number;
  private tracks: TrackMeta[] = [];
  private nextTrackName = 1;
  private nextClipId = 1;
  private bpm = 120;
  private playing = false;
  /// Lazily populated by scanPlugins().
  private pluginCache: NativePluginInfo[] | null = null;

  constructor(sampleRate = 44100, bufferSize = 512) {
    this.sampleRate = sampleRate;
    this.bufferSize = bufferSize;
  }

  /** Whether the native addon is loaded. */
  isAvailable(): boolean {
    return addon !== null;
  }

  /** Scan system directories for VST3/CLAP plugins (cached after first call). */
  scanPlugins(force = false): NativePluginInfo[] {
    if (!addon) return [];
    if (this.pluginCache !== null && !force) return this.pluginCache;
    try {
      this.pluginCache = addon.scanPlugins(this.sampleRate, this.bufferSize);
    } catch (e) {
      console.error('[engine] scanPlugins failed:', e);
      this.pluginCache = [];
    }
    return this.pluginCache;
  }

  /** Whether a session has been created (at least one track added). */
  isInitialized(): boolean {
    return this.session !== null;
  }

  // --- Track management ---------------------------------------------------

  addTrack(instrumentPath?: string): TrackMeta | null {
    if (!addon) return this.addMockTrack(instrumentPath ?? null);

    try {
      const backend = instrumentPath
        ? addon.create(instrumentPath, this.sampleRate, this.bufferSize)
        : addon.createGain(this.sampleRate); // silent placeholder

      let trackId: number;

      if (!this.session) {
        // First track -- create the session
        this.session = addon.Session.create(backend, this.sampleRate, this.bufferSize);
        try {
          this.session.start();
        } catch (startErr) {
          console.error('[engine] Session.start() failed — audio device unavailable:', startErr);
          this.session = null;
          return null;
        }
        this.session.setTempo(this.bpm);
        trackId = 0;
        console.log(`[engine] session started (sr=${this.sampleRate}, buf=${this.bufferSize})`);
      } else {
        // Subsequent tracks
        trackId = this.session.addTrack(backend, 0xFFFF);
      }
      console.log(`[engine] addTrack id=${trackId} instrument=${instrumentPath ?? '(silent placeholder)'}`);

      const meta: TrackMeta = {
        id: trackId,
        name: `Track ${this.nextTrackName++}`,
        color: COLORS[this.tracks.length % COLORS.length],
        volume: 0,
        pan: 0,
        muted: false,
        solo: false,
        instrumentPath: instrumentPath ?? null,
        inserts: [],
        clips: [],
      };
      this.tracks.push(meta);
      return meta;
    } catch (e) {
      console.error('[engine] addTrack failed:', e);
      return null;
    }
  }

  removeTrack(trackId: number): boolean {
    if (!this.session) return false;

    try {
      this.session.removeTrack(trackId);
      this.tracks = this.tracks.filter((t) => t.id !== trackId);
      return true;
    } catch (e) {
      console.error('[engine] removeTrack failed:', e);
      return false;
    }
  }

  loadInstrument(trackId: number, path: string): boolean {
    if (!addon || !this.session) {
      console.warn('[engine] loadInstrument: no session');
      return false;
    }
    const track = this.tracks.find((t) => t.id === trackId);
    if (!track) {
      console.error(`[engine] loadInstrument: track ${trackId} not found`);
      return false;
    }
    try {
      const backend = addon.create(path, this.sampleRate, this.bufferSize);
      this.session.swapTrackBackend(trackId, backend);
      track.instrumentPath = path;
      console.log(`[engine] loadInstrument track=${trackId} path=${path}`);
      return true;
    } catch (e) {
      console.error(`[engine] loadInstrument failed (track=${trackId}, path=${path}):`, e);
      return false;
    }
  }

  /** Load a MIDI file onto a track as a clip and stage it on the audio thread. */
  loadMidi(trackId: number, filePath: string, fileName: string): ClipMeta | null {
    const track = this.tracks.find((t) => t.id === trackId);
    if (!track) {
      console.error(`[engine] loadMidi: track ${trackId} not found`);
      return null;
    }

    // Stage the file on the audio thread sequencer. Transport state stays
    // unchanged — the user presses Play to start playback.
    if (this.session) {
      try {
        this.session.loadMidi(filePath);
        console.log(`[engine] loadMidi: ${fileName} -> sequencer (track ${trackId})`);
      } catch (e) {
        console.error('[engine] loadMidi (native) failed:', e);
        return null;
      }
    } else {
      console.warn(`[engine] loadMidi: ${fileName} accepted but no session yet (add a track first)`);
    }

    const clip: ClipMeta = {
      id: this.nextClipId++,
      name: fileName.replace(/\.midi?$/i, ''),
      startBar: 0,
      lengthBars: 8, // duration parsing is a future polish item
    };

    track.clips = [...track.clips, clip];
    return clip;
  }

  // --- Transport ----------------------------------------------------------

  play(): void {
    if (!this.session) {
      console.warn('[engine] play() called but no session exists — add a track first');
      return;
    }
    const trackCount = this.tracks.length;
    const withInstrument = this.tracks.filter((t) => t.instrumentPath !== null).length;
    console.log(`[engine] play (tracks=${trackCount}, with instrument=${withInstrument})`);
    this.session.play();
    this.playing = true;
  }

  stop(): void {
    console.log('[engine] stop');
    this.session?.stopPlayback();
    this.playing = false;
  }

  setBpm(bpm: number): void {
    this.bpm = bpm;
    this.session?.setTempo(bpm);
  }

  isPlaying(): boolean {
    if (this.session) {
      try {
        return this.session.isPlaying();
      } catch {
        return this.playing;
      }
    }
    return this.playing;
  }

  // --- MIDI ---------------------------------------------------------------

  noteOn(channel: number, note: number, velocity: number): void {
    this.session?.noteOn(channel, note, velocity);
  }

  noteOff(channel: number, note: number): void {
    this.session?.noteOff(channel, note);
  }

  // --- Mixer controls -----------------------------------------------------

  setTrackVolume(trackId: number, db: number): void {
    // Convert dB to linear for the native API (0.0-1.0 linear scale).
    // The mixer expects linear gain, not dB.
    const linear = db <= -96 ? 0 : Math.pow(10, db / 20);
    this.session?.setTrackVolume(trackId, linear);

    const track = this.tracks.find((t) => t.id === trackId);
    if (track) track.volume = db;
  }

  setTrackPan(trackId: number, pan: number): void {
    this.session?.setTrackPan(trackId, pan);

    const track = this.tracks.find((t) => t.id === trackId);
    if (track) track.pan = pan;
  }

  setTrackMute(trackId: number, muted: boolean): void {
    this.session?.setTrackMute(trackId, muted);

    const track = this.tracks.find((t) => t.id === trackId);
    if (track) track.muted = muted;
  }

  setTrackSolo(trackId: number, solo: boolean): void {
    this.session?.setTrackSolo(trackId, solo);

    const track = this.tracks.find((t) => t.id === trackId);
    if (track) track.solo = solo;
  }

  setMasterVolume(db: number): void {
    const linear = db <= -96 ? 0 : Math.pow(10, db / 20);
    this.session?.setMasterVolume(linear);
  }

  // --- Inserts ------------------------------------------------------------

  addInsert(trackId: number, effectType: string): InsertMeta | null {
    if (!addon || !this.session) return null;

    const factory = EFFECT_FACTORIES[effectType];
    if (!factory) {
      console.error(`[engine] Unknown effect type: ${effectType}`);
      return null;
    }

    try {
      const effect = factory(addon, this.sampleRate);
      // Snapshot params BEFORE addInsert consumes the backend.
      const params = snapshotParams(effect);
      const insertId = this.session.addInsert(trackId, effect);

      const insert: InsertMeta = {
        id: insertId,
        name: effectType,
        bypassed: false,
        params,
      };
      const track = this.tracks.find((t) => t.id === trackId);
      if (track) {
        track.inserts.push(insert);
      }
      return insert;
    } catch (e) {
      console.error('[engine] addInsert failed:', e);
      return null;
    }
  }

  removeInsert(trackId: number, insertId: number): void {
    this.session?.removeInsert(trackId, insertId);

    const track = this.tracks.find((t) => t.id === trackId);
    if (track) {
      track.inserts = track.inserts.filter((ins) => ins.id !== insertId);
    }
  }

  setInsertParam(trackId: number, insertId: number, paramId: number, value: number): void {
    this.session?.setInsertParam(trackId, insertId, paramId, value);
    // Mirror locally so getState() returns up-to-date values for late joiners.
    const track = this.tracks.find((t) => t.id === trackId);
    const insert = track?.inserts.find((i) => i.id === insertId);
    const param = insert?.params.find((p) => p.id === paramId);
    if (param) param.value = value;
  }

  // --- Metering -----------------------------------------------------------

  getTrackLevels(trackId: number): { peakL: number; peakR: number } {
    if (!this.session) return { peakL: 0, peakR: 0 };

    try {
      return this.session.trackLevels(trackId);
    } catch {
      return { peakL: 0, peakR: 0 };
    }
  }

  getMasterLevels(): { peakL: number; peakR: number } {
    if (!this.session) return { peakL: 0, peakR: 0 };

    try {
      return this.session.masterLevels();
    } catch {
      return { peakL: 0, peakR: 0 };
    }
  }

  trackCount(): number {
    return this.tracks.length;
  }

  // --- State snapshot for new clients -------------------------------------

  getState(): { tracks: TrackMeta[]; bpm: number; playing: boolean } {
    return {
      tracks: this.tracks,
      bpm: this.bpm,
      playing: this.isPlaying(),
    };
  }

  // --- Shutdown -----------------------------------------------------------

  shutdown(): void {
    if (this.session) {
      try {
        this.session.shutdown();
      } catch (e) {
        console.error('[engine] shutdown error:', e);
      }
      this.session = null;
    }
    this.tracks = [];
  }

  // --- Mock mode (no addon) -----------------------------------------------

  private addMockTrack(instrumentPath: string | null): TrackMeta {
    const meta: TrackMeta = {
      id: this.tracks.length,
      name: `Track ${this.nextTrackName++}`,
      color: COLORS[this.tracks.length % COLORS.length],
      volume: 0,
      pan: 0,
      muted: false,
      solo: false,
      instrumentPath,
      inserts: [],
      clips: [],
    };
    this.tracks.push(meta);
    return meta;
  }
}
