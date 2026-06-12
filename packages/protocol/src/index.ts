// Client -> Server commands
export type Command =
  | { type: 'transport.play' }
  | { type: 'transport.pause' }
  | { type: 'transport.stop' }
  | { type: 'transport.set_bpm'; bpm: number }
  | { type: 'transport.set_loop'; looping: boolean }
  | { type: 'transport.set_metronome'; enabled: boolean }
  | { type: 'master.set_volume'; db: number }
  | { type: 'plugins.scan'; force?: boolean }

  // --- Default (master) instrument -----------------------------------------
  // Sets the SF2/VST3/CLAP that plays every channel without an override.
  | { type: 'default.set_instrument'; path: string }

  // --- Per-channel override -----------------------------------------------
  // Promote a MIDI channel to its own backend (or replace an existing one).
  | { type: 'channel.set_override'; channel: number; path: string }
  // Drop an override → channel reverts to the default instrument.
  | { type: 'channel.remove_override'; channel: number }
  // Override-track mixer controls (only meaningful when an override exists).
  | { type: 'channel.set_volume'; channel: number; db: number }
  | { type: 'channel.set_pan'; channel: number; pan: number }
  | { type: 'channel.set_mute'; channel: number; muted: boolean }
  | { type: 'channel.set_solo'; channel: number; solo: boolean }
  | { type: 'channel.set_color'; channel: number; color: string | null }
  // Pick a GM preset for a channel by sending a Program Change. Works for
  // any backend that responds to MIDI PC (SF2 always does; VST3/CLAP varies).
  // The MIDI file's own PC events will override this on the next event.
  | { type: 'channel.set_program'; channel: number; program: number }

  // --- Inserts on the channel-override track -------------------------------
  | { type: 'insert.add'; channel: number; effectType: string }
  | { type: 'insert.remove'; channel: number; insertId: number }
  | { type: 'insert.set_bypass'; channel: number; insertId: number; bypassed: boolean }
  | { type: 'insert.set_param'; channel: number; insertId: number; paramId: number; value: number }
  | { type: 'transport.seek'; ticks: number }
  | { type: 'send_bus.add'; effectType: string }
  | { type: 'send_bus.set_param'; busId: number; paramId: number; value: number }
  | { type: 'channel.set_send_level'; channel: number; busId: number; level: number };

// Server -> Client events
export type ServerEvent =
  | { type: 'state.init'; project: ProjectState }
  | { type: 'transport.state'; playing: boolean; position: number }
  | { type: 'transport.tempo_changed'; bpm: number }
  | { type: 'transport.loop_changed'; looping: boolean }
  | { type: 'transport.metronome_changed'; enabled: boolean }
  | { type: 'master.updated'; volumeDb: number }
  | { type: 'midi.loaded'; midi: MidiState }
  | { type: 'default.instrument_changed'; instrumentPath: string | null; needsPatch?: boolean }
  | { type: 'channel.override_added'; override: ChannelOverrideState; needsPatch?: boolean }
  | { type: 'channel.override_removed'; channel: number }
  | { type: 'channel.updated'; channel: number; volume?: number; pan?: number; muted?: boolean; solo?: boolean; color?: string | null; userProgram?: number | null }
  | { type: 'insert.added'; channel: number; insert: InsertState }
  | { type: 'insert.removed'; channel: number; insertId: number }
  | { type: 'insert.bypass_changed'; channel: number; insertId: number; bypassed: boolean }
  | { type: 'send_bus.added'; bus: SendBusView }
  | { type: 'channel.send_level_changed'; channel: number; busId: number; level: number }
  | { type: 'plugins.list'; plugins: PluginInfo[] }
  | { type: 'error'; message: string };

// --- State shapes -----------------------------------------------------------

/** Master bus state — volume only for now (UI builds master mute on top). */
export interface MasterState {
  volumeDb: number;
}

/** Whole project snapshot, sent on connect. */
export interface ProjectState {
  bpm: number;
  playing: boolean;
  looping: boolean;
  metronomeEnabled: boolean;
  master: MasterState;
  defaultInstrumentPath: string | null;
  /**
   * Patch name parsed from the default instrument's captured state when
   * available (Spectrasonics plug-ins embed it as plain XML; most other
   * plug-ins don't expose it through any standard surface). Absent when
   * unknown or when no state has been captured.
   */
  defaultPatchName?: string;
  midi: MidiState | null;
  overrides: ChannelOverrideState[];
  sendBuses: SendBusView[];
}

/** A send / aux bus — one effect (reverb, delay, etc.) with channel sends. */
export interface SendBusView {
  id: number;
  name: string;
  effectType: string;
  level: number;
  params: ParamMeta[];
}

/** Information about the currently-loaded MIDI file. */
export interface MidiState {
  name: string;
  tempoBpm: number | null;
  timeSignature: [number, number] | null;
  lengthBars: number;
  /** Clip length in MIDI ticks — the progress-bar denominator. */
  totalTicks: number;
  /** MIDI resolution in ticks per quarter note. */
  ticksPerBeat: number;
  channels: MidiChannelInfo[];
}

export interface MidiChannelInfo {
  /** 0-based MIDI channel (wire). */
  channel: number;
  /** 1-based human number (1..16). */
  displayNumber: number;
  /** TrackName meta event from the MIDI track that owns this channel's notes. */
  trackName?: string;
  /** First Program Change observed on this channel (0..127), or absent. */
  program?: number;
}

export interface ChannelOverrideState {
  channel: number;
  instrumentPath: string;
  instrumentName: string;
  /** See [`ProjectState.defaultPatchName`]. */
  patchName?: string;
  volume: number;   // dB
  /** Stereo pan in [-1.0, 1.0]; 0.0 = center. */
  pan: number;
  muted: boolean;
  solo: boolean;
  inserts: InsertState[];
  /** Per-bus send level (indexed by bus id; 0 = silent). */
  sendLevels: number[];
  /** Optional user-assigned color (CSS hex, e.g. "#4a90d9"). */
  color?: string | null;
}

export interface PluginInfo {
  name: string;
  path: string;
  /// "Sf2" | "Vst3" | "Clap" (from Rust enum debug)
  format: string;
  /** False for effect-only plug-ins (e.g. FX-Omnisphere) — instrument
   *  pickers hide these. Optional for older backends. */
  isInstrument?: boolean;
}

export interface InsertState {
  id: number;
  name: string;
  bypassed: boolean;
  params: ParamMeta[];
}

/// Metadata + current value for a single backend parameter.
export interface ParamMeta {
  id: number;
  name: string;
  group: string;
  min: number;
  max: number;
  default: number;
  /// 0 = continuous, >0 = discrete steps.
  stepCount: number;
  /// Current value (mirrors what the audio thread holds).
  value: number;
}
