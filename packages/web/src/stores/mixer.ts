import { create } from 'zustand';
import type { TrackState } from '@moonlitt/protocol';
import { TRACK_COLORS } from '@moonlitt/protocol';

export interface Insert {
  id: number;
  name: string;
  bypassed: boolean;
}

export interface Clip {
  id: number;
  name: string;
  startBar: number;
  lengthBars: number;
}

export interface Track {
  id: number;
  name: string;
  color: string;
  volume: number;
  pan: number;
  muted: boolean;
  solo: boolean;
  peakL: number;
  peakR: number;
  instrumentPath: string | null;
  instrumentName: string | null;
  inserts: Insert[];
  clips: Clip[];
}

interface MixerStore {
  tracks: Track[];
  selectedTrackId: number | null;
  masterVolume: number;
  masterPeakL: number;
  masterPeakR: number;

  // Actions
  selectTrack(trackId: number | null): void;
  setMasterVolume(db: number): void;
  addTrack(trackId: number, name: string, color: string): void;
  removeTrack(trackId: number): void;
  addClip(trackId: number, clip: Clip): void;
  initTracks(trackStates: TrackState[]): void;
  updateMeters(data: Float32Array): void;
  setTrackVolume(trackId: number, db: number): void;
  setTrackPan(trackId: number, pan: number): void;
  setTrackMute(trackId: number, muted: boolean): void;
  setTrackSolo(trackId: number, solo: boolean): void;
}

function trackFromState(state: TrackState): Track {
  return {
    id: state.id,
    name: state.name,
    color: state.color,
    volume: state.volume,
    pan: state.pan,
    muted: state.muted,
    solo: state.solo,
    peakL: 0,
    peakR: 0,
    instrumentPath: state.instrumentPath,
    instrumentName: null,
    inserts: state.inserts.map((ins) => ({
      id: ins.id,
      name: ins.name,
      bypassed: ins.bypassed,
    })),
    clips: [],
  };
}

function updateTrack(
  tracks: readonly Track[],
  trackId: number,
  updater: (track: Track) => Track,
): Track[] {
  return tracks.map((t) => (t.id === trackId ? updater(t) : t));
}

export const useMixerStore = create<MixerStore>((set, get) => ({
  tracks: [],
  selectedTrackId: null,
  masterVolume: 0,
  masterPeakL: 0,
  masterPeakR: 0,

  selectTrack(trackId: number | null) {
    set({ selectedTrackId: trackId });
  },

  setMasterVolume(db: number) {
    set({ masterVolume: db });
  },

  addTrack(trackId: number, name: string, color: string) {
    const { tracks } = get();
    const newTrack: Track = {
      id: trackId,
      name,
      color: color || TRACK_COLORS[tracks.length % TRACK_COLORS.length],
      volume: 0,
      pan: 0,
      muted: false,
      solo: false,
      peakL: 0,
      peakR: 0,
      instrumentPath: null,
      instrumentName: null,
      inserts: [],
      clips: [],
    };
    set({ tracks: [...tracks, newTrack] });
  },

  removeTrack(trackId: number) {
    const { tracks, selectedTrackId } = get();
    set({
      tracks: tracks.filter((t) => t.id !== trackId),
      selectedTrackId: selectedTrackId === trackId ? null : selectedTrackId,
    });
  },

  addClip(trackId: number, clip: Clip) {
    set({
      tracks: updateTrack(get().tracks, trackId, (t) => ({
        ...t,
        clips: [...t.clips, clip],
      })),
    });
  },

  initTracks(trackStates: TrackState[]) {
    set({
      tracks: trackStates.map(trackFromState),
      selectedTrackId: trackStates.length > 0 ? trackStates[0].id : null,
    });
  },

  // Binary meter data layout:
  // [track0_peakL, track0_peakR, track1_peakL, track1_peakR, ..., masterPeakL, masterPeakR]
  updateMeters(data: Float32Array) {
    const { tracks } = get();
    const trackCount = tracks.length;

    // Validate data length: 2 floats per track + 2 for master
    if (data.length < trackCount * 2 + 2) {
      return;
    }

    const updatedTracks = tracks.map((track, i) => ({
      ...track,
      peakL: data[i * 2],
      peakR: data[i * 2 + 1],
    }));

    set({
      tracks: updatedTracks,
      masterPeakL: data[trackCount * 2],
      masterPeakR: data[trackCount * 2 + 1],
    });
  },

  setTrackVolume(trackId: number, db: number) {
    set({ tracks: updateTrack(get().tracks, trackId, (t) => ({ ...t, volume: db })) });
  },

  setTrackPan(trackId: number, pan: number) {
    set({ tracks: updateTrack(get().tracks, trackId, (t) => ({ ...t, pan })) });
  },

  setTrackMute(trackId: number, muted: boolean) {
    set({ tracks: updateTrack(get().tracks, trackId, (t) => ({ ...t, muted })) });
  },

  setTrackSolo(trackId: number, solo: boolean) {
    set({ tracks: updateTrack(get().tracks, trackId, (t) => ({ ...t, solo })) });
  },
}));
