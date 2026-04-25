/**
 * Command protocol handler.
 *
 * Maps incoming WebSocket command JSON (from @moonlitt/protocol) to
 * EngineManager method calls. Returns a ServerEvent to broadcast back, or
 * null. The server's index.ts wraps this with broadcasting logic so a
 * single command can fan out to multiple events when needed.
 */

import type { Command, ServerEvent } from '@moonlitt/protocol';
import type { EngineManager } from './engine.js';

export function handleCommand(engine: EngineManager, cmd: Command): ServerEvent | ServerEvent[] | null {
  switch (cmd.type) {
    // --- Transport --------------------------------------------------------

    case 'transport.play':
      engine.play();
      return { type: 'transport.state', playing: true, position: 0 };

    case 'transport.stop':
      engine.stop();
      return { type: 'transport.state', playing: false, position: 0 };

    case 'transport.set_bpm':
      engine.setBpm(cmd.bpm);
      return { type: 'transport.tempo_changed', bpm: cmd.bpm };

    // --- Master -----------------------------------------------------------

    case 'master.set_volume':
      engine.setMasterVolume(cmd.db);
      return null;

    // --- Default instrument ----------------------------------------------

    case 'default.set_instrument': {
      const ok = engine.setDefaultInstrument(cmd.path);
      if (!ok) return { type: 'error', message: `Failed to load default instrument: ${cmd.path}` };
      return { type: 'default.instrument_changed', instrumentPath: cmd.path };
    }

    // --- Per-channel overrides -------------------------------------------

    case 'channel.set_override': {
      const ov = engine.setChannelOverride(cmd.channel, cmd.path);
      if (!ov) return { type: 'error', message: `Failed to set override on channel ${cmd.channel + 1}` };
      return {
        type: 'channel.override_added',
        override: {
          channel: ov.channel,
          instrumentPath: ov.instrumentPath,
          instrumentName: ov.instrumentName,
          volume: ov.volume,
          muted: ov.muted,
          solo: ov.solo,
          inserts: ov.inserts,
        },
      };
    }

    case 'channel.remove_override': {
      const ok = engine.removeChannelOverride(cmd.channel);
      if (!ok) return { type: 'error', message: `No override on channel ${cmd.channel + 1}` };
      return { type: 'channel.override_removed', channel: cmd.channel };
    }

    case 'channel.set_volume':
      engine.setChannelVolume(cmd.channel, cmd.db);
      return { type: 'channel.updated', channel: cmd.channel, volume: cmd.db };

    case 'channel.set_mute':
      engine.setChannelMute(cmd.channel, cmd.muted);
      return { type: 'channel.updated', channel: cmd.channel, muted: cmd.muted };

    case 'channel.set_solo':
      engine.setChannelSolo(cmd.channel, cmd.solo);
      return { type: 'channel.updated', channel: cmd.channel, solo: cmd.solo };

    // --- Inserts (on override tracks) ------------------------------------

    case 'insert.add': {
      const insert = engine.addInsert(cmd.channel, cmd.effectType);
      if (!insert) return { type: 'error', message: `Failed to add ${cmd.effectType} on channel ${cmd.channel + 1}` };
      return { type: 'insert.added', channel: cmd.channel, insert };
    }

    case 'insert.remove':
      engine.removeInsert(cmd.channel, cmd.insertId);
      return { type: 'insert.removed', channel: cmd.channel, insertId: cmd.insertId };

    case 'insert.set_param':
      engine.setInsertParam(cmd.channel, cmd.insertId, cmd.paramId, cmd.value);
      return null;

    // --- Plugin discovery ------------------------------------------------

    case 'plugins.scan': {
      const plugins = engine.scanPlugins(cmd.force ?? false);
      return { type: 'plugins.list', plugins };
    }

    default: {
      const exhaustive: never = cmd;
      return { type: 'error', message: `Unknown command: ${(exhaustive as { type: string }).type}` };
    }
  }
}
