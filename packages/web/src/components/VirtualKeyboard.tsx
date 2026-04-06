import { useCallback, useRef } from 'react';
import { useSessionStore } from '../stores/session';
import { useKeyboard } from '../hooks/useKeyboard';

const NOTE_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
const BLACK_KEY_INDICES = new Set([1, 3, 6, 8, 10]); // C#, D#, F#, G#, A#

// 2 octaves: C3 (48) to B4 (71), plus C5 (72)
const START_NOTE = 48; // C3
const END_NOTE = 72;   // C5 inclusive
const DEFAULT_VELOCITY = 100;

function noteToName(note: number): string {
  const name = NOTE_NAMES[note % 12];
  const octave = Math.floor(note / 12) - 1;
  return `${name}${octave}`;
}

interface KeyDef {
  note: number;
  name: string;
  isBlack: boolean;
}

function buildKeys(): KeyDef[] {
  const keys: KeyDef[] = [];
  for (let note = START_NOTE; note <= END_NOTE; note++) {
    const semitone = note % 12;
    keys.push({
      note,
      name: noteToName(note),
      isBlack: BLACK_KEY_INDICES.has(semitone),
    });
  }
  return keys;
}

const ALL_KEYS = buildKeys();
const WHITE_KEYS = ALL_KEYS.filter((k) => !k.isBlack);
const BLACK_KEYS = ALL_KEYS.filter((k) => k.isBlack);

// Calculate black key position based on which white key it sits next to
function blackKeyLeftPosition(note: number): number {
  const whiteKeysBefore = ALL_KEYS
    .filter((k) => !k.isBlack && k.note < note)
    .length;

  // Black key sits between the previous white key and current position
  // Offset slightly to center on the boundary
  const whiteKeyWidthPercent = 100 / WHITE_KEYS.length;
  return whiteKeysBefore * whiteKeyWidthPercent - whiteKeyWidthPercent * 0.3;
}

export function VirtualKeyboard() {
  const send = useSessionStore((s) => s.send);
  const { octave, activeNotes } = useKeyboard();
  const mouseNoteRef = useRef<number | null>(null);

  const handleMouseDown = useCallback(
    (note: number) => {
      mouseNoteRef.current = note;
      send({ type: 'midi.note_on', channel: 0, note, velocity: DEFAULT_VELOCITY });
    },
    [send],
  );

  const handleMouseUp = useCallback(() => {
    if (mouseNoteRef.current !== null) {
      send({ type: 'midi.note_off', channel: 0, note: mouseNoteRef.current });
      mouseNoteRef.current = null;
    }
  }, [send]);

  const handleMouseLeave = useCallback(() => {
    if (mouseNoteRef.current !== null) {
      send({ type: 'midi.note_off', channel: 0, note: mouseNoteRef.current });
      mouseNoteRef.current = null;
    }
  }, [send]);

  return (
    <div className="h-16 bg-daw-surface border-t border-daw-border flex items-center px-3 gap-3 select-none">
      {/* Octave indicator */}
      <div className="flex flex-col items-center gap-0.5 text-xs text-[#888] shrink-0">
        <span>OCT</span>
        <span className="text-[#e0e0e0] font-mono">C{octave}</span>
        <div className="flex gap-1 text-[10px]">
          <span title="Z = octave down">Z-</span>
          <span title="X = octave up">X+</span>
        </div>
      </div>

      {/* Piano keyboard */}
      <div
        className="relative flex-1 h-12"
        onMouseLeave={handleMouseLeave}
        onMouseUp={handleMouseUp}
      >
        {/* White keys */}
        <div className="flex h-full">
          {WHITE_KEYS.map((key) => {
            const isActive = activeNotes.has(key.note);
            return (
              <button
                key={key.note}
                type="button"
                onMouseDown={() => handleMouseDown(key.note)}
                className={`flex-1 h-full border-r border-[#333] rounded-b-sm transition-colors ${
                  isActive ? 'bg-gray-300' : 'bg-white hover:bg-gray-100'
                }`}
                title={key.name}
              >
                {key.note % 12 === 0 && (
                  <span className="text-[9px] text-[#999] block mt-auto pt-7">
                    {key.name}
                  </span>
                )}
              </button>
            );
          })}
        </div>

        {/* Black keys */}
        {BLACK_KEYS.map((key) => {
          const left = blackKeyLeftPosition(key.note);
          const isActive = activeNotes.has(key.note);
          const widthPercent = (100 / WHITE_KEYS.length) * 0.6;
          return (
            <button
              key={key.note}
              type="button"
              onMouseDown={(e) => {
                e.stopPropagation();
                handleMouseDown(key.note);
              }}
              className={`absolute top-0 rounded-b-sm transition-colors ${
                isActive ? 'bg-gray-500' : 'bg-gray-800 hover:bg-gray-700'
              }`}
              style={{
                left: `${left}%`,
                width: `${widthPercent}%`,
                height: '60%',
              }}
              title={key.name}
            />
          );
        })}
      </div>
    </div>
  );
}
