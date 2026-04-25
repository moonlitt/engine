/**
 * Engine wrapper for moonlitt-node native addon.
 *
 * Architecture: ONE master mixer track that holds the default instrument
 * and listens to the union of channels NOT overridden. ZERO or more
 * "override" tracks, each pinned to a single MIDI channel.
 *
 * Master mask = 0xFFFF & ~(union of overridden channel bits).
 */

// ---------------------------------------------------------------------------
// Native addon import
// ---------------------------------------------------------------------------

interface NativeBackend {
  sampleRate(): number;
  paramCount(): number;
  setParam(id: number, value: number): void;
  getParam(id: number): number | null;
  paramInfo(index: number): NativeParamInfo | null;
  paramDisplay(id: number, value: number): string | null;
  isConsumed(): boolean;
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
  setTrackChannelMask(trackId: number, channelMask: number): void;
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
  trackLevels(trackId: number): { peakL: number; peakR: number };
  masterLevels(): { peakL: number; peakR: number };
  trackCount(): number;
  droppedEvents(): number;
  shutdown(): void;
}

interface NativePluginInfo {
  name: string;
  path: string;
  format: string;
}

interface NativeChannelInfo {
  channel: number;
  displayNumber: number;
  trackName?: string;
  program?: number;
}

interface NativeMidiInfo {
  channels: NativeChannelInfo[];
  trackCount: number;
  lengthBars: number;
  tempoBpm: number | null;
  timeSignature: number[] | null;
}

interface NativeAddon {
  create(path: string, sampleRate: number, bufferSize: number): NativeBackend;
  scanPlugins(sampleRate: number, bufferSize: number): NativePluginInfo[];
  supportedFormats(): string[];
  analyzeMidi(path: string): NativeMidiInfo;
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
  createConvolver(sampleRate: number): NativeBackend;
  Session: { create(backend: NativeBackend, sampleRate: number, bufferSize: number): NativeSession };
}

let addon: NativeAddon | null = null;

try {
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
// Effect factory lookup
// ---------------------------------------------------------------------------

const EFFECT_FACTORIES: Record<string, (a: NativeAddon, sr: number) => NativeBackend> = {
  eq:                    (a, sr) => a.createEq(sr),
  compressor:            (a, sr) => a.createCompressor(sr),
  reverb:                (a, sr) => a.createReverb(sr),
  'dattorro-reverb':     (a, sr) => a.createDattorroReverb(sr),
  limiter:               (a, sr) => a.createLimiter(sr),
  gate:                  (a, sr) => a.createGate(sr),
  deesser:               (a, sr) => a.createDeesser(sr),
  'stereo-delay':        (a, sr) => a.createStereoDelay(sr),
  delay:                 (a, sr) => a.createStereoDelay(sr),
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
// Server-side state shapes (kept separate from protocol so we can carry
// internal-only fields like nativeTrackId).
// ---------------------------------------------------------------------------

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

interface OverrideTrack {
  channel: number;             // 0-based MIDI channel
  nativeTrackId: number;       // mixer track ID
  instrumentPath: string;
  instrumentName: string;
  volume: number;
  muted: boolean;
  solo: boolean;
  inserts: InsertMeta[];
}

interface MidiState {
  name: string;
  path: string;
  tempoBpm: number | null;
  timeSignature: [number, number] | null;
  lengthBars: number;
  channels: NativeChannelInfo[];
}

const EFFECT_FRIENDLY_NAMES: Record<string, string> = {
  eq: 'EQ', compressor: 'Compressor', reverb: 'Reverb',
  'dattorro-reverb': 'Dattorro Reverb', limiter: 'Limiter', gate: 'Gate',
  deesser: 'De-esser', 'stereo-delay': 'Stereo Delay', delay: 'Delay',
  chorus: 'Chorus', flanger: 'Flanger', phaser: 'Phaser', tremolo: 'Tremolo',
  saturator: 'Saturator', bitcrusher: 'Bitcrusher',
  'multiband-compressor': 'Multiband Compressor', 'auto-filter': 'Auto Filter',
  'pitch-shifter': 'Pitch Shifter', gain: 'Gain', 'stereo-width': 'Stereo Width',
};

function snapshotParams(backend: NativeBackend): ParamMeta[] {
  const out: ParamMeta[] = [];
  const count = backend.paramCount();
  for (let i = 0; i < count; i++) {
    const info = backend.paramInfo(i);
    if (!info) continue;
    const value = backend.getParam(info.id);
    out.push({
      id: info.id, name: info.name, group: info.group,
      min: info.min, max: info.max, default: info.default,
      stepCount: info.stepCount, value: value ?? info.default,
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

  // ONE master track. nativeTrackId is null until the first MIDI/instrument
  // load creates the audio session.
  private masterTrackId: number | null = null;
  private defaultInstrumentPath: string | null = null;

  private overrides: OverrideTrack[] = [];
  private nextInsertId = 1;

  private midi: MidiState | null = null;
  private bpm = 120;
  private playing = false;
  private pluginCache: NativePluginInfo[] | null = null;

  constructor(sampleRate = 44100, bufferSize = 512) {
    this.sampleRate = sampleRate;
    this.bufferSize = bufferSize;
  }

  isAvailable(): boolean { return addon !== null; }

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

  /** Snapshot of the project for the `state.init` event. */
  snapshot(): {
    bpm: number; playing: boolean;
    defaultInstrumentPath: string | null;
    midi: MidiState | null;
    overrides: Array<Omit<OverrideTrack, 'nativeTrackId'>>;
  } {
    return {
      bpm: this.bpm,
      playing: this.playing,
      defaultInstrumentPath: this.defaultInstrumentPath,
      midi: this.midi,
      overrides: this.overrides.map(({ nativeTrackId: _, ...rest }) => rest),
    };
  }

  // --- Bootstrap: ensure session + master exist --------------------------

  private ensureMaster(): boolean {
    if (!addon) return false;
    if (this.masterTrackId !== null) return true;
    try {
      // Bootstrap with the chosen default instrument if set, else a silent
      // gain placeholder. The first instrument load swaps it in.
      const initialBackend = this.defaultInstrumentPath
        ? addon.create(this.defaultInstrumentPath, this.sampleRate, this.bufferSize)
        : addon.createGain(this.sampleRate);
      this.session = addon.Session.create(initialBackend, this.sampleRate, this.bufferSize);
      try {
        this.session.start();
      } catch (startErr) {
        console.error('[engine] Session.start() failed — audio device unavailable:', startErr);
        this.session = null;
        return false;
      }
      this.session.setTempo(this.bpm);
      this.masterTrackId = 0;
      console.log(`[engine] session started (sr=${this.sampleRate}, buf=${this.bufferSize})`);
      console.log(`[engine] master track=0 mask=0xFFFF instrument=${this.defaultInstrumentPath ?? '(silent placeholder)'}`);
      return true;
    } catch (e) {
      console.error('[engine] ensureMaster failed:', e);
      return false;
    }
  }

  /** Recompute master mask = 0xFFFF & ~(union of overridden channels). */
  private syncMasterMask(): void {
    if (!this.session || this.masterTrackId === null) return;
    let masterMask = 0xFFFF;
    for (const o of this.overrides) {
      masterMask &= ~(1 << o.channel);
    }
    this.session.setTrackChannelMask(this.masterTrackId, masterMask);
    console.log(`[engine] master mask → 0x${masterMask.toString(16)}`);
  }

  // --- Default instrument -------------------------------------------------

  setDefaultInstrument(path: string): boolean {
    if (!addon) return false;
    if (!this.ensureMaster() || this.session === null || this.masterTrackId === null) return false;
    try {
      const backend = addon.create(path, this.sampleRate, this.bufferSize);
      this.session.swapTrackBackend(this.masterTrackId, backend);
      this.defaultInstrumentPath = path;
      console.log(`[engine] default instrument → ${path}`);
      return true;
    } catch (e) {
      console.error('[engine] setDefaultInstrument failed:', e);
      return false;
    }
  }

  // --- MIDI loading -------------------------------------------------------

  loadMidi(filePath: string, fileName: string): MidiState | null {
    if (!addon) return null;
    let info: NativeMidiInfo;
    try {
      info = addon.analyzeMidi(filePath);
    } catch (e) {
      console.error('[engine] analyzeMidi failed:', e);
      return null;
    }

    if (!this.ensureMaster() || !this.session) return null;

    // Auto-apply tempo from the MIDI file (the user can still override
    // afterwards via transport.set_bpm).
    if (info.tempoBpm !== null && Number.isFinite(info.tempoBpm)) {
      this.bpm = info.tempoBpm;
      this.session.setTempo(this.bpm);
    }

    try {
      this.session.loadMidi(filePath);
    } catch (e) {
      console.error('[engine] session.loadMidi failed:', e);
      return null;
    }

    const ts: [number, number] | null =
      info.timeSignature && info.timeSignature.length === 2
        ? [info.timeSignature[0], info.timeSignature[1]]
        : null;

    this.midi = {
      name: fileName,
      path: filePath,
      tempoBpm: info.tempoBpm,
      timeSignature: ts,
      lengthBars: info.lengthBars,
      channels: info.channels,
    };
    console.log(
      `[engine] loadMidi: ${fileName} channels=[${info.channels.map(c => c.displayNumber).join(',')}] ` +
      `bpm=${info.tempoBpm?.toFixed(1) ?? '?'} ts=${ts ? ts.join('/') : '?'} bars=${info.lengthBars.toFixed(1)}`,
    );
    return this.midi;
  }

  // --- Channel overrides --------------------------------------------------

  setChannelOverride(channel: number, path: string): OverrideTrack | null {
    if (!addon) return null;
    if (!this.ensureMaster() || this.session === null) return null;
    if (channel < 0 || channel > 15) return null;

    try {
      const backend = addon.create(path, this.sampleRate, this.bufferSize);
      const instrumentName = path.split('/').pop() ?? path;

      const existing = this.overrides.find((o) => o.channel === channel);
      if (existing) {
        // Replace backend on the existing override track.
        this.session.swapTrackBackend(existing.nativeTrackId, backend);
        existing.instrumentPath = path;
        existing.instrumentName = instrumentName;
        console.log(`[engine] override ch=${channel + 1} (existing) → ${path}`);
        return existing;
      }

      const mask = 1 << channel;
      const nativeTrackId = this.session.addTrack(backend, mask);
      const ov: OverrideTrack = {
        channel, nativeTrackId,
        instrumentPath: path,
        instrumentName,
        volume: 0, muted: false, solo: false,
        inserts: [],
      };
      this.overrides.push(ov);
      this.syncMasterMask();
      console.log(`[engine] override ch=${channel + 1} created → ${path}`);
      return ov;
    } catch (e) {
      console.error('[engine] setChannelOverride failed:', e);
      return null;
    }
  }

  removeChannelOverride(channel: number): boolean {
    if (!this.session) return false;
    const idx = this.overrides.findIndex((o) => o.channel === channel);
    if (idx < 0) return false;
    const ov = this.overrides[idx];
    try {
      this.session.removeTrack(ov.nativeTrackId);
      this.overrides.splice(idx, 1);
      this.syncMasterMask();
      console.log(`[engine] override ch=${channel + 1} removed`);
      return true;
    } catch (e) {
      console.error('[engine] removeChannelOverride failed:', e);
      return false;
    }
  }

  setChannelVolume(channel: number, db: number): boolean {
    const ov = this.overrides.find((o) => o.channel === channel);
    if (!ov || !this.session) return false;
    this.session.setTrackVolume(ov.nativeTrackId, dbToLinear(db));
    ov.volume = db;
    return true;
  }

  setChannelMute(channel: number, muted: boolean): boolean {
    const ov = this.overrides.find((o) => o.channel === channel);
    if (!ov || !this.session) return false;
    this.session.setTrackMute(ov.nativeTrackId, muted);
    ov.muted = muted;
    return true;
  }

  setChannelSolo(channel: number, solo: boolean): boolean {
    const ov = this.overrides.find((o) => o.channel === channel);
    if (!ov || !this.session) return false;
    this.session.setTrackSolo(ov.nativeTrackId, solo);
    ov.solo = solo;
    return true;
  }

  // --- Inserts (on override tracks only) ----------------------------------

  addInsert(channel: number, effectType: string): InsertMeta | null {
    if (!addon || !this.session) return null;
    const ov = this.overrides.find((o) => o.channel === channel);
    if (!ov) {
      console.warn(`[engine] addInsert: no override for channel ${channel + 1}`);
      return null;
    }
    const factory = EFFECT_FACTORIES[effectType];
    if (!factory) {
      console.error(`[engine] unknown effect type: ${effectType}`);
      return null;
    }
    try {
      const backend = factory(addon, this.sampleRate);
      const params = snapshotParams(backend);
      const insertId = this.session.addInsert(ov.nativeTrackId, backend);
      const meta: InsertMeta = {
        id: insertId,
        name: EFFECT_FRIENDLY_NAMES[effectType] ?? effectType,
        bypassed: false,
        params,
      };
      ov.inserts.push(meta);
      this.nextInsertId = Math.max(this.nextInsertId, insertId + 1);
      return meta;
    } catch (e) {
      console.error('[engine] addInsert failed:', e);
      return null;
    }
  }

  removeInsert(channel: number, insertId: number): boolean {
    if (!this.session) return false;
    const ov = this.overrides.find((o) => o.channel === channel);
    if (!ov) return false;
    this.session.removeInsert(ov.nativeTrackId, insertId);
    ov.inserts = ov.inserts.filter((i) => i.id !== insertId);
    return true;
  }

  setInsertParam(channel: number, insertId: number, paramId: number, value: number): boolean {
    if (!this.session) return false;
    const ov = this.overrides.find((o) => o.channel === channel);
    if (!ov) return false;
    this.session.setInsertParam(ov.nativeTrackId, insertId, paramId, value);
    const insert = ov.inserts.find((i) => i.id === insertId);
    if (insert) {
      const param = insert.params.find((p) => p.id === paramId);
      if (param) param.value = value;
    }
    return true;
  }

  // --- Per-channel program selection (manual override of MIDI PC) -------

  /** Send a Program Change for the given MIDI channel via the audio thread. */
  setChannelProgram(channel: number, program: number): boolean {
    if (!this.session) return false;
    if (channel < 0 || channel > 15) return false;
    if (program < 0 || program > 127) return false;
    try {
      this.session.programChange(channel, program);
      console.log(`[engine] program ch=${channel + 1} → ${program}`);
      return true;
    } catch (e) {
      console.error('[engine] setChannelProgram failed:', e);
      return false;
    }
  }

  // --- Transport ----------------------------------------------------------

  play(): void {
    if (!this.session) {
      console.warn('[engine] play() called but no session yet — load a MIDI or pick a default instrument first');
      return;
    }
    console.log('[engine] play — snapshot:');
    console.log(`[engine]   master track=${this.masterTrackId} mask=auto instrument=${this.defaultInstrumentPath ?? '(silent placeholder)'}`);
    for (const o of this.overrides) {
      console.log(`[engine]   override ch=${o.channel + 1} instrument=${o.instrumentPath} mute=${o.muted} solo=${o.solo} inserts=${o.inserts.length}`);
    }
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

  setMasterVolume(db: number): void {
    if (!this.session) return;
    this.session.setMasterVolume(dbToLinear(db));
  }

  isPlaying(): boolean {
    if (this.session) {
      try { return this.session.isPlaying(); } catch { return this.playing; }
    }
    return this.playing;
  }

  getBpm(): number { return this.bpm; }
  getMidi(): MidiState | null { return this.midi; }
  getOverrides(): OverrideTrack[] { return this.overrides; }
  getDefaultInstrumentPath(): string | null { return this.defaultInstrumentPath; }

  // --- Metering -----------------------------------------------------------

  /** Returns interleaved [masterL, masterR, overrideL_0, overrideR_0, …]. */
  meterSnapshot(): Float32Array {
    if (!this.session) return new Float32Array(2);
    const buf = new Float32Array(2 + this.overrides.length * 2);
    const m = this.session.masterLevels();
    buf[0] = m.peakL; buf[1] = m.peakR;
    for (let i = 0; i < this.overrides.length; i++) {
      const ov = this.overrides[i];
      try {
        const lvl = this.session.trackLevels(ov.nativeTrackId);
        buf[2 + i * 2] = lvl.peakL;
        buf[2 + i * 2 + 1] = lvl.peakR;
      } catch { /* track gone, ignore */ }
    }
    return buf;
  }
}

// ---------------------------------------------------------------------------

function dbToLinear(db: number): number {
  return Math.pow(10, db / 20);
}
