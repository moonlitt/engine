# Moonlitt Piano — Stardew Valley mod

The original moonlitt vision: blocks in a game playing real instruments.
Flute Blocks and Drum Blocks route through the moonlitt engine instead of
the vanilla sound bank — a GM piano out of the box, or **any sound you
design in the moonlitt desktop app**, Keyscape patches included.

## What it does

- **Flute Block** `(O)464` — plays a real instrument note. The block's
  vanilla tuning (right-click to cycle, 24 semitones) maps to MIDI
  pitch, so existing flute-block songs keep their melodies. The Ginger
  Island flute puzzle keeps working (`OnFlutePlayed` is preserved).
- **Drum Block** `(O)463` — the 7 kit pieces map to GM percussion on
  channel 9 (kick, snare, hats, tom, crash, clap).
- `moonlitt_test` console command — plays an arpeggio to verify audio.

## Sound sources (priority order)

1. **`SessionPath`** in `config.json` — a `.mlsession` saved from the
   moonlitt desktop app. Instruments, captured plug-in states (e.g. a
   Keyscape patch), mixer and sends come up exactly as designed.
   Design your piano in the DAW, hear it on the farm.
2. **`Sf2Path`** — explicit GM SoundFont.
3. Auto-scan `~/Library/Audio/Sounds/Banks` (GeneralUser preferred).

## Build & install

```bash
# 1. Engine (universal binary so it loads under Rosetta or native):
cargo build --release -p moonlitt-capi
cargo build --release -p moonlitt-capi --target x86_64-apple-darwin
mkdir -p target/universal
lipo -create target/release/libmoonlitt.dylib \
     target/x86_64-apple-darwin/release/libmoonlitt.dylib \
     -output target/universal/libmoonlitt.dylib

# 2. Mod (auto-deploys into the game's Mods/ via ModBuildConfig):
cd examples/stardew-piano/MoonlittPiano
dotnet build
```

Launch the game through SMAPI as usual. The SMAPI console logs
`moonlitt engine running` when the engine is live.

## config.json

| key | default | meaning |
| --- | --- | --- |
| `SessionPath` | `""` | `.mlsession` from the desktop app (wins when valid) |
| `Sf2Path` | `""` | explicit SoundFont; empty = auto-scan Banks dir |
| `Volume` | `0.9` | master volume 0–1 |
| `FluteBaseNote` | `48` | MIDI note for a fully-down-tuned flute block |
| `FluteChannel` | `0` | MIDI channel for flute blocks |
| `NoteSeconds` | `2.0` | ring time before the scheduled note-off |

## Architecture notes

- The C# binding sources compile directly into the mod assembly; the
  mod resolves the `moonlitt` DllImport to the `libmoonlitt.dylib`
  shipped in its own folder (`NativeLibrary.SetDllImportResolver`).
- Patches are Harmony prefixes that replicate the vanilla decompile's
  gating (1 s per-block cooldown, dialogue/diagonal checks) and visual
  feedback (shake + scale pop), then skip the vanilla cue. If the
  engine fails to start, every patch falls through to vanilla.
- All note scheduling is fire-and-forget via the engine's
  sample-accurate delayed note-off — no game-thread timers.
