/**
 * Command protocol handler.
 *
 * Maps incoming WebSocket command JSON (from @moonlitt/protocol) to
 * EngineManager method calls. Returns a ServerEvent to send back, or null.
 */

import type { Command, ServerEvent } from '@moonlitt/protocol';
import type { EngineManager } from './engine.js';

/**
 * Handle a single command from a WebSocket client.
 * Returns a ServerEvent to send back (or null for fire-and-forget commands).
 */
export function handleCommand(engine: EngineManager, cmd: Command): ServerEvent | null {
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
      return null;

    // --- Tracks -----------------------------------------------------------

    case 'track.add': {
      const track = engine.addTrack(cmd.instrumentPath);
      if (!track) {
        return { type: 'error', message: 'Failed to add track' };
      }
      return {
        type: 'track.added',
        trackId: track.id,
        name: track.name,
        color: track.color,
      };
    }

    case 'track.remove': {
      const removed = engine.removeTrack(cmd.trackId);
      if (!removed) {
        return { type: 'error', message: `Failed to remove track ${cmd.trackId}` };
      }
      return { type: 'track.removed', trackId: cmd.trackId };
    }

    case 'track.load_instrument': {
      const loaded = engine.loadInstrument(cmd.trackId, cmd.path);
      if (!loaded) {
        return { type: 'error', message: 'Instrument hot-swap not yet supported. Remove and re-add the track.' };
      }
      return null;
    }

    // --- Track mixer controls ---------------------------------------------

    case 'track.set_volume':
      engine.setTrackVolume(cmd.trackId, cmd.db);
      return null;

    case 'track.set_pan':
      engine.setTrackPan(cmd.trackId, cmd.pan);
      return null;

    case 'track.set_mute':
      engine.setTrackMute(cmd.trackId, cmd.muted);
      return null;

    case 'track.set_solo':
      engine.setTrackSolo(cmd.trackId, cmd.solo);
      return null;

    // --- Master -----------------------------------------------------------

    case 'master.set_volume':
      engine.setMasterVolume(cmd.db);
      return null;

    // --- MIDI -------------------------------------------------------------

    case 'midi.note_on':
      engine.noteOn(cmd.channel, cmd.note, cmd.velocity);
      return null;

    case 'midi.note_off':
      engine.noteOff(cmd.channel, cmd.note);
      return null;

    case 'midi.load_file':
      // MIDI file loading not yet implemented in the engine wrapper.
      return { type: 'error', message: 'MIDI file loading not yet implemented' };

    // --- Insert effects ---------------------------------------------------

    case 'insert.add': {
      const insertId = engine.addInsert(cmd.trackId, cmd.effectType);
      if (insertId === null) {
        return { type: 'error', message: `Failed to add ${cmd.effectType} insert` };
      }
      return null;
    }

    case 'insert.remove':
      engine.removeInsert(cmd.trackId, cmd.insertId);
      return null;

    case 'insert.set_param':
      engine.setInsertParam(cmd.trackId, cmd.insertId, cmd.paramId, cmd.value);
      return null;

    default: {
      const exhaustive: never = cmd;
      return { type: 'error', message: `Unknown command: ${(exhaustive as any).type}` };
    }
  }
}
