import { create } from 'zustand';
import type {
  ChannelOverrideState,
  InsertState,
  MidiState,
  ParamMeta,
  ProjectState,
  SendBusView,
} from '@moonlitt/protocol';

/**
 * Single store for the whole project: default instrument, MIDI metadata,
 * per-channel overrides. Mirrors the server's snapshot.
 */
interface ProjectStore {
  defaultInstrumentPath: string | null;
  defaultPatchName: string | null;
  /** Master bus volume in dB. -∞ (silent) modelled as -60 in the UI. */
  masterVolumeDb: number;
  midi: MidiState | null;
  overrides: ChannelOverrideState[];
  sendBuses: SendBusView[];

  setProject(p: ProjectState): void;
  setMasterVolume(db: number): void;
  setDefaultInstrument(path: string | null): void;
  setMidi(midi: MidiState): void;

  upsertOverride(ov: ChannelOverrideState): void;
  removeOverride(channel: number): void;
  updateChannel(channel: number, patch: { volume?: number; pan?: number; muted?: boolean; solo?: boolean; color?: string | null }): void;

  addInsert(channel: number, insert: InsertState): void;
  removeInsert(channel: number, insertId: number): void;
  setInsertBypass(channel: number, insertId: number, bypassed: boolean): void;
  setInsertParam(channel: number, insertId: number, paramId: number, value: number): void;

  addSendBus(bus: SendBusView): void;
  setChannelSendLevel(channel: number, busId: number, level: number): void;
}

export const useProjectStore = create<ProjectStore>((set) => ({
  defaultInstrumentPath: null,
  defaultPatchName: null,
  masterVolumeDb: 0,
  midi: null,
  overrides: [],
  sendBuses: [],

  setProject(p) {
    set({
      defaultInstrumentPath: p.defaultInstrumentPath,
      defaultPatchName: p.defaultPatchName ?? null,
      masterVolumeDb: p.master?.volumeDb ?? 0,
      midi: p.midi,
      overrides: p.overrides,
      sendBuses: p.sendBuses ?? [],
    });
  },

  setMasterVolume(db) {
    set({ masterVolumeDb: db });
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
    // Drop undefined fields so they don't blank existing values when the
    // patch carries a single dimension (e.g. only `mute`). `color === null`
    // is meaningful (clear), so distinguish from undefined.
    const cleaned: Partial<ChannelOverrideState> = {};
    if (patch.volume !== undefined) cleaned.volume = patch.volume;
    if (patch.pan !== undefined) cleaned.pan = patch.pan;
    if (patch.muted !== undefined) cleaned.muted = patch.muted;
    if (patch.solo !== undefined) cleaned.solo = patch.solo;
    if (patch.color !== undefined) cleaned.color = patch.color;
    set((s) => ({
      overrides: s.overrides.map((o) =>
        o.channel === channel ? { ...o, ...cleaned } : o,
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

  setInsertBypass(channel, insertId, bypassed) {
    set((s) => ({
      overrides: s.overrides.map((o) =>
        o.channel !== channel
          ? o
          : {
              ...o,
              inserts: o.inserts.map((i) =>
                i.id === insertId ? { ...i, bypassed } : i,
              ),
            },
      ),
    }));
  },

  addSendBus(bus) {
    set((s) => {
      // Mixer also pushes 0.0 onto every track's send_levels — mirror so
      // the UI's sliders start at zero rather than undefined.
      const overrides = s.overrides.map((o) => ({
        ...o,
        sendLevels: [...o.sendLevels, 0],
      }));
      return { sendBuses: [...s.sendBuses, bus], overrides };
    });
  },

  setChannelSendLevel(channel, busId, level) {
    set((s) => ({
      overrides: s.overrides.map((o) => {
        if (o.channel !== channel) return o;
        const next = [...o.sendLevels];
        while (next.length <= busId) next.push(0);
        next[busId] = level;
        return { ...o, sendLevels: next };
      }),
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
