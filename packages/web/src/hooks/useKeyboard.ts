import { useEffect, useRef, useCallback, useState } from 'react';
import { useSessionStore } from '../stores/session';

// Base key -> MIDI note mapping (starting at C4 = 60 by default)
const KEY_TO_OFFSET: Record<string, number> = {
  a: 0,   // C
  w: 1,   // C#
  s: 2,   // D
  e: 3,   // D#
  d: 4,   // E
  f: 5,   // F
  t: 6,   // F#
  g: 7,   // G
  y: 8,   // G#
  h: 9,   // A
  u: 10,  // A#
  j: 11,  // B
  k: 12,  // C (next octave)
};

const DEFAULT_VELOCITY = 100;
const MIN_OCTAVE = 0;
const MAX_OCTAVE = 8;

interface UseKeyboardResult {
  octave: number;
  activeNotes: ReadonlySet<number>;
}

export function useKeyboard(): UseKeyboardResult {
  const send = useSessionStore((s) => s.send);
  const octaveRef = useRef(4); // C4 = middle C = MIDI 60
  const activeNotesRef = useRef(new Set<number>());
  const heldKeysRef = useRef(new Set<string>());

  // Force re-render when state changes
  const forceUpdate = useForceUpdate();

  const noteOn = useCallback(
    (note: number) => {
      if (note < 0 || note > 127) return;
      if (activeNotesRef.current.has(note)) return;

      activeNotesRef.current = new Set(activeNotesRef.current).add(note);
      send({ type: 'midi.note_on', channel: 0, note, velocity: DEFAULT_VELOCITY });
      forceUpdate();
    },
    [send, forceUpdate],
  );

  const noteOff = useCallback(
    (note: number) => {
      if (!activeNotesRef.current.has(note)) return;

      const next = new Set(activeNotesRef.current);
      next.delete(note);
      activeNotesRef.current = next;
      send({ type: 'midi.note_off', channel: 0, note });
      forceUpdate();
    },
    [send, forceUpdate],
  );

  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      // Ignore when focused on an input
      if (isInputFocused(e)) return;

      const key = e.key.toLowerCase();

      // Octave shift
      if (key === 'z') {
        if (octaveRef.current > MIN_OCTAVE) {
          octaveRef.current -= 1;
          forceUpdate();
        }
        return;
      }
      if (key === 'x') {
        if (octaveRef.current < MAX_OCTAVE) {
          octaveRef.current += 1;
          forceUpdate();
        }
        return;
      }

      // Prevent key repeat
      if (heldKeysRef.current.has(key)) return;

      const offset = KEY_TO_OFFSET[key];
      if (offset === undefined) return;

      heldKeysRef.current.add(key);
      const note = octaveRef.current * 12 + offset;
      noteOn(note);
    }

    function handleKeyUp(e: KeyboardEvent) {
      const key = e.key.toLowerCase();

      if (!heldKeysRef.current.has(key)) return;
      heldKeysRef.current.delete(key);

      const offset = KEY_TO_OFFSET[key];
      if (offset === undefined) return;

      const note = octaveRef.current * 12 + offset;
      noteOff(note);
    }

    window.addEventListener('keydown', handleKeyDown);
    window.addEventListener('keyup', handleKeyUp);

    return () => {
      window.removeEventListener('keydown', handleKeyDown);
      window.removeEventListener('keyup', handleKeyUp);
    };
  }, [noteOn, noteOff, forceUpdate]);

  return {
    octave: octaveRef.current,
    activeNotes: activeNotesRef.current,
  };
}

function isInputFocused(e: KeyboardEvent): boolean {
  const target = e.target as HTMLElement;
  return (
    target.tagName === 'INPUT' ||
    target.tagName === 'TEXTAREA' ||
    target.tagName === 'SELECT' ||
    target.isContentEditable
  );
}

function useForceUpdate() {
  const [, setState] = useState(0);
  return useCallback(() => setState((n) => n + 1), []);
}
