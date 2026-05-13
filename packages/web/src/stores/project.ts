import { create } from 'zustand';
import type {
  ChannelOverrideState,
  InsertState,
  MidiState,
  ParamMeta,
  ProjectState,
} from '@moonlitt/protocol';

/**
 * Single store for the whole project: default instrument, MIDI metadata,
 * per-channel overrides. Mirrors the server's snapshot.
 */
interface ProjectStore {
  defaultInstrumentPath: string | null;
  defaultPatchName: string | null;
  midi: MidiState | null;
  overrides: ChannelOverrideState[];

  setProject(p: ProjectState): void;
  setDefaultInstrument(path: string | null): void;
  setMidi(midi: MidiState): void;

  upsertOverride(ov: ChannelOverrideState): void;
  removeOverride(channel: number): void;
  updateChannel(channel: number, patch: { volume?: number; muted?: boolean; solo?: boolean }): void;

  addInsert(channel: number, insert: InsertState): void;
  removeInsert(channel: number, insertId: number): void;
  setInsertParam(channel: number, insertId: number, paramId: number, value: number): void;
}

export const useProjectStore = create<ProjectStore>((set) => ({
  defaultInstrumentPath: null,
  defaultPatchName: null,
  midi: null,
  overrides: [],

  setProject(p) {
    set({
      defaultInstrumentPath: p.defaultInstrumentPath,
      defaultPatchName: p.defaultPatchName ?? null,
      midi: p.midi,
      overrides: p.overrides,
    });
  },

  setDefaultInstrument(path) {
    set({ defaultInstrumentPath: path, defaultPatchName: null });
  },

  setMidi(midi) {
    set({ midi });
  },

  upsertOverride(ov) {
    set((s) => {
      const next = s.overrides.filter((o) => o.channel !== ov.channel);
      next.push(ov);
      next.sort((a, b) => a.channel - b.channel);
      return { overrides: next };
    });
  },

  removeOverride(channel) {
    set((s) => ({ overrides: s.overrides.filter((o) => o.channel !== channel) }));
  },

  updateChannel(channel, patch) {
    set((s) => ({
      overrides: s.overrides.map((o) =>
        o.channel === channel ? { ...o, ...patch } : o,
      ),
    }));
  },

  addInsert(channel, insert) {
    set((s) => ({
      overrides: s.overrides.map((o) =>
        o.channel === channel ? { ...o, inserts: [...o.inserts, insert] } : o,
      ),
    }));
  },

  removeInsert(channel, insertId) {
    set((s) => ({
      overrides: s.overrides.map((o) =>
        o.channel === channel ? { ...o, inserts: o.inserts.filter((i) => i.id !== insertId) } : o,
      ),
    }));
  },

  setInsertParam(channel, insertId, paramId, value) {
    set((s) => ({
      overrides: s.overrides.map((o) => {
        if (o.channel !== channel) return o;
        return {
          ...o,
          inserts: o.inserts.map((i) =>
            i.id !== insertId
              ? i
              : { ...i, params: i.params.map((p) => (p.id === paramId ? { ...p, value } : p)) },
          ),
        };
      }),
    }));
  },
}));

export type { ChannelOverrideState, MidiState, InsertState, ParamMeta };
