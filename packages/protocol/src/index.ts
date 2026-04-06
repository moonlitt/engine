// Client -> Server commands
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

// Server -> Client events
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
