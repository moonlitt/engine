// Client -> Server commands
export type Command =
  | { type: 'transport.play' }
  | { type: 'transport.stop' }
  | { type: 'transport.set_bpm'; bpm: number }
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
  | { type: 'channel.set_mute'; channel: number; muted: boolean }
  | { type: 'channel.set_solo'; channel: number; solo: boolean }
  // Pick a GM preset for a channel by sending a Program Change. Works for
  // any backend that responds to MIDI PC (SF2 always does; VST3/CLAP varies).
  // The MIDI file's own PC events will override this on the next event.
  | { type: 'channel.set_program'; channel: number; program: number }

  // --- Inserts on the channel-override track -------------------------------
  | { type: 'insert.add'; channel: number; effectType: string }
  | { type: 'insert.remove'; channel: number; insertId: number }
  | { type: 'insert.set_param'; channel: number; insertId: number; paramId: number; value: number };

// Server -> Client events
export type ServerEvent =
  | { type: 'state.init'; project: ProjectState }
  | { type: 'transport.state'; playing: boolean; position: number }
  | { type: 'transport.tempo_changed'; bpm: number }
  | { type: 'midi.loaded'; midi: MidiState }
  | { type: 'default.instrument_changed'; instrumentPath: string | null }
  | { type: 'channel.override_added'; override: ChannelOverrideState }
  | { type: 'channel.override_removed'; channel: number }
  | { type: 'channel.updated'; channel: number; volume?: number; muted?: boolean; solo?: boolean; userProgram?: number | null }
  | { type: 'insert.added'; channel: number; insert: InsertState }
  | { type: 'insert.removed'; channel: number; insertId: number }
  | { type: 'plugins.list'; plugins: PluginInfo[] }
  | { type: 'error'; message: string };

// --- State shapes -----------------------------------------------------------

/** Whole project snapshot, sent on connect. */
export interface ProjectState {
  bpm: number;
  playing: boolean;
  defaultInstrumentPath: string | null;
  midi: MidiState | null;
  overrides: ChannelOverrideState[];
}

/** Information about the currently-loaded MIDI file. */
export interface MidiState {
  name: string;
  tempoBpm: number | null;
  timeSignature: [number, number] | null;
  lengthBars: number;
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
  volume: number;   // dB
  muted: boolean;
  solo: boolean;
  inserts: InsertState[];
}

export interface PluginInfo {
  name: string;
  path: string;
  /// "Sf2" | "Vst3" | "Clap" (from Rust enum debug)
  format: string;
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
