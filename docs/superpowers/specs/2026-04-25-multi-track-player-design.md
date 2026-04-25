# Multi-track player redesign

**Date:** 2026-04-25
**Status:** Approved (auto mode, requirements clear)
**Owner:** wangyan

## Goal

The single-track testbed (commit 6db8e7c) is too constrained: the user wants to
audition **multiple sources side-by-side** against the same or different MIDI
files, while still keeping the UI focused enough that every action is visible
without hunting.

Three concrete pain points to fix:

1. **No sound.** Pressing Play after loading instrument + MIDI produces no
   audio. Root cause unknown — needs diagnosis (see "No-sound investigation"
   below).
2. **SF2 not in picker.** `moonlitt-engine::scan_plugins()` has a `// TODO:
   scan for SF2 files` and only ever returns VST3/CLAP entries. SF2 users have
   to paste a path manually.
3. **No per-track view.** The testbed shows only `tracks[0]`. The user wants a
   DAW-style list where each track is independently editable: source, MIDI,
   M/S, volume, effect chain.

## Non-goals

- Timeline / arrange / clip editing. A track has at most one MIDI clip
  starting at bar 0; that's enough for auditioning.
- Mixer panel, virtual keyboard, transport ruler chrome — none of these
  serve the audition use-case.
- Preset save/load. Sessions are throwaway.
- Bar-grid scrolling, playhead drawing.

## UX layout

Single screen, vertical track list. Header pinned, tracks scroll if many.

```
┌─────────────────────────────────────────────────────────────┐
│ moonlitt player    ● connected    ▶ Play  ■ Stop            │
│ POS 1.1  BPM 120  Master ━━━━●━━━━━ -3.0dB                  │
├─────────────────────────────────────────────────────────────┤
│ ┌── Track 1 (red) ──────────────────────────────────────┐   │
│ │ ▌ Track 1   [Pick instrument…]   M S   ━━━●━━━ 0dB    │   │
│ │ ┌─ MIDI ──────────────────────────────────────────┐   │   │
│ │ │  Drop a .mid file here, or click to choose      │   │   │
│ │ └─────────────────────────────────────────────────┘   │   │
│ │ ▾ Effects (0)         [+ Add]                         │   │
│ └───────────────────────────────────────────────────────┘   │
│ ┌── Track 2 (green) ────────────────────────────────────┐   │
│ │ ▌ Track 2  Surge.vst3 [Change…]  M S   ━●━━━━━ -6dB   │   │
│ │ ┌─ MIDI ──────────────────────────────────────────┐   │   │
│ │ │  ✓ Prelude1.mid                                 │   │   │
│ │ └─────────────────────────────────────────────────┘   │   │
│ │ ▾ Effects (1)         [+ Add]                         │   │
│ │   • Dattorro Reverb   [×]                             │   │
│ │     Decay   ━━━━●━━━━ 0.7                             │   │
│ │     Damping ━━●━━━━━━ 0.3                             │   │
│ └───────────────────────────────────────────────────────┘   │
│ [+ Add Track]                                               │
└─────────────────────────────────────────────────────────────┘
```

### Why this layout

- Each track is a self-contained card → user knows the boundary of "what affects
  this source".
- Source picker, MIDI drop zone, M/S, volume, and effects are all visible at
  once — no expand/collapse for the primary actions.
- Effects fold open by default (so the user can see what's running), can be
  collapsed to save vertical space.
- "Add Track" sits at the bottom of the list — discoverable, unobtrusive.

### Compared to the testbed (6db8e7c)

The testbed had four global cards. The new design promotes "Instrument",
"MIDI", "Effects" to per-track cards. Transport stays global at the top.

## SF2 discovery

Add SF2 scanning to `moonlitt-engine::scan_plugins`. Search dirs in priority
order (each entry recursively walked, max depth 4):

1. `$MOONLITT_SF2_DIR` (env var, colon-separated, if set)
2. `~/Library/Audio/Sounds/Banks` (macOS standard)
3. `~/Documents/Soundfonts`
4. Project root `tests/` and `deps/oxisynth/testdata/` (for dev convenience)

Cap at 100 SF2 entries per scan to avoid pathological dirs.

Picker UI gets a permanent "Or paste a path" input (already exists) so any
non-scanned location still works.

## Per-track instrument swap

Wiring already exists end-to-end:

- Server: `engine.loadInstrument(trackId, path)` → `session.swapTrackBackend()`
- Audio: `Mixer::replace_track_backend()` swaps backend, all-notes-off the old.

UI just needs to let the user trigger it per track. Instrument-selector modal
already opens with a `trackId` via `useUiStore` — each track row's "Pick…" /
"Change…" button just calls `openInstrumentSelector(track.id)`.

## No-sound investigation

Before/while implementing the new UI, verify the playback chain:

1. Add a console log on the server when `transport.play` is received and
   when `loadMidi` stages a sequencer, so we can see the order at runtime.
2. Confirm `Session.start()` actually opens cpal — currently failures may
   be swallowed. If `start()` returns/throws an error, surface it as an
   `error` event to the client and show in UI.
3. Verify default channel mask `0xFFFF` actually routes channel-1 MIDI to the
   track. (The MIDI files in `examples/midi-test/` are channel 1.)
4. Manual end-to-end: server log + client log + cpal device check.

The fix for whatever is wrong is in scope for this design.

## Components / files

**Web (`packages/web/src/`):**

- `components/PlayerView.tsx` (new) — replaces `TestbedView` as the App body.
  Renders header + track list + add-track button.
- `components/TrackCard.tsx` (new) — one track row. Composes:
  - `InstrumentField` — current name + Pick/Change button
  - `MidiField` — drop zone or loaded clip pill
  - `MasterControls` — M/S toggles + volume slider
  - `EffectsSection` — collapsible insert chain
- `components/Header.tsx` (new) — global transport + master volume.
- `components/EffectsSection.tsx` (extracted from current testbed)
- `components/InsertRow.tsx` (extracted from current testbed)
- `components/InstrumentSelector.tsx` — keep as-is, opened from store.
- `App.tsx` — render `<PlayerView />` + `<InstrumentSelector />`.
- Delete: `TestbedView.tsx`, `ArrangeView.tsx`, `Mixer.tsx`, `TrackHeader.tsx`,
  `TrackInspector.tsx`, `TimelineRuler.tsx`, `MidiClip.tsx`, `Playhead.tsx`,
  `VirtualKeyboard.tsx`, `TransportBar.tsx`. (All replaced by the smaller
  PlayerView surface.)

**Server (`packages/server/src/`):**

- `engine.ts` — log on play / loadMidi / loadInstrument; surface
  `Session.start()` errors as protocol errors.

**Engine (`crates/moonlitt-engine/src/`):**

- `engine.rs::scan_plugins` — implement SF2 scan (replace TODO).
- New module: `engine/sf2_scan.rs` — directory walker with the rules above.

## Data flow (one round trip)

User clicks Pick on Track 2:

1. `<TrackCard>` calls `useUiStore.openInstrumentSelector(2)`
2. `<App>` re-renders the modal with `targetTrackId=2`
3. User clicks an SF2 entry → `onLoad(path)` in App → sends
   `track.load_instrument` over WS
4. Server: `engine.loadInstrument(2, path)` → `session.swapTrackBackend(2, …)`
   → broadcasts `track.instrument_changed`
5. Client: mixer store updates `tracks[trackIdx].instrumentPath` → `<TrackCard>`
   re-renders with new instrument name.

## Error handling

- Instrument load failure → `error` event → show inline red text under the
  source field (auto-clears on next successful load).
- MIDI upload failure → already handled in `MidiField` (was `MidiCard`).
- Session start failure (cpal device unavailable) → `error` event surfaced as
  a banner under the header so the user knows audio is dead.

## Testing

Manual end-to-end is enough for this UI. Concrete checks before declaring
done:

1. Open browser → connection dot turns green → 1 track auto-creates.
2. Pick instrument → list shows VST3 *and* SF2 entries.
3. Pick SF2 from list → track shows it; load a `.mid` → track shows clip.
4. Press Play → audible sound.
5. Add a 2nd track → pick a different instrument → load same MIDI → A/B by
   muting.
6. Add Dattorro Reverb on track 1 → audible reverb tail.

If step 4 fails, the no-sound investigation has not concluded — keep digging.

## Out of scope (explicit non-goals worth reiterating)

- Timeline view, clip warping, multiple clips per track
- Mixer with channel strips
- Per-track sequencer (one global sequencer drives all tracks via channel
  routing — sufficient for audition)
- Recording, virtual keyboard, MIDI input devices
