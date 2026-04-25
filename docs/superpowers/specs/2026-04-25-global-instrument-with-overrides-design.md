# Global instrument + per-channel override + use MIDI metadata

**Date:** 2026-04-25
**Status:** Draft (auto mode — present then implement)
**Owner:** wangyan
**Supersedes:** 2026-04-25-multi-track-player-design.md

## Why this changes

The previous design (one DAW track per MIDI channel, each requiring its
own instrument) had two problems:

1. **It fights how SF2 + GM works.** A General-MIDI SoundFont like
   `GeneralUser_GS` contains 128+ presets. The MIDI file's own Program
   Change events on each channel select the right preset (piano, bass,
   strings…). One oxisynth backend with mask `0xFFFF` plays the whole
   file correctly because the backend dispatches the events to the right
   internal voice per channel. Splitting into per-channel tracks with no
   instrument loaded produced silence — the most common complaint.
2. **It puts effort on the user that the MIDI file already encodes.**
   Tempo, time signature, channel-to-instrument hints, and track names
   are all in the file. The previous design ignored every one of them.

The new model: **one global instrument, with per-channel override.** Like
CSS inheritance or `Object.assign(globalDefaults, perTrackOverride)`.

## UX

```
┌──────────────────────────────────────────────────────────────────┐
│ moonlitt player    ● connected   ▶ Play  ■ Stop   POS 1.1  120.0 BPM │
├──────────────────────────────────────────────────────────────────┤
│ MIDI: voyage.mid  ·  3 channels  ·  120 BPM  ·  4/4  ·  98 bars  │
│       [Replace…]                                                  │
├──────────────────────────────────────────────────────────────────┤
│ Default instrument                                                │
│ 🎹 [GeneralUser_GS]  [Change…]                                   │
│   Plays every MIDI channel that doesn't have an override below.  │
├──────────────────────────────────────────────────────────────────┤
│ Channels in this MIDI                                             │
│                                                                   │
│ ┌─ Ch 1 · "Piano" ──────────────────────────────────────────┐   │
│ │ Inherits default · M ◯  S ◯  vol ━━●━━━ 0dB              │   │
│ │ [Override instrument…]                                     │   │
│ └────────────────────────────────────────────────────────────┘   │
│ ┌─ Ch 10 · "Drums" ─────────────────────────────────────────┐   │
│ │ Override 🥁 [DrumKit.sf2]  [Change…] [×]  M ◯ S ◯ vol …  │   │
│ │ ▾ Effects (1)  · Reverb                                    │   │
│ └────────────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────────┘
```

Key affordances:

- **One global instrument picker** at the top — covers most of what the
  user needs for GM playback.
- **One row per channel** that the MIDI actually uses. Each row defaults
  to "Inherits default". Clicking "Override instrument…" promotes that
  channel to a real backend the user picks.
- **Tempo, time signature, channel count, length** all surfaced from the
  MIDI metadata. BPM auto-applies on upload (the user can still edit it).

## Architecture

Two kinds of mixer tracks:

1. **Master track** — one instance, holds the global instrument. Mask is
   computed as `0xFFFF & ~(union of overridden-channel bits)`. So when
   nothing is overridden, master receives every channel; when channel 10
   is overridden, master's mask becomes `0xFBFF`.
2. **Override track** — one per overridden channel. Mask is exactly
   `1 << channel`. Holds whichever backend the user picked for that
   channel.

The mask invariant is enforced server-side: every time an override is
added or removed, the server recomputes and pushes the master's mask.

The mixer already routes by channel mask, so this falls out of the
existing engine. The only new mixer op is the existing
`set_track_channel_mask` (just shipped).

## MIDI metadata, used

Extend the existing `analyzeMidi(path)` to return:

```ts
{
  channels: number[];       // already there
  trackCount: number;       // already there
  lengthBars: number;       // already there
  tempoBpm: number | null;  // first MetaMessage::Tempo we see
  timeSignature: [num, den] | null;  // first MetaMessage::TimeSignature
  channelNames: Record<number, string>;
    // best-effort: for each channel, the TrackName from the MIDI track
    // that contains its events, OR the inferred GM patch name from the
    // first Program Change on that channel
}
```

On upload the server:

1. Calls `analyzeMidi`.
2. If `tempoBpm` is set → broadcasts `transport.tempo_changed` and pushes
   to the audio thread (`session.setTempo`).
3. Updates the channel-row UI with the channel names and instrument
   hints.

## Components / files

**Engine (Rust):**
- `moonlitt-node::engine::analyze_midi` — extend to return tempo, time
  signature, per-channel inferred names.

**Server (TypeScript):**
- `engine.ts`:
  - Replace `loadMidiMultitrack` with a flow that:
    1. Ensures a single master track exists (mask 0xFFFF) loaded with the
       last-used default instrument (or empty until the user picks one).
    2. Updates the master's mask to `0xFFFF & ~overrides`.
    3. Stages the new sequencer.
  - New methods:
    - `setDefaultInstrument(path)` — swaps the master backend.
    - `setChannelOverride(channel, path)` — adds or replaces an override
      track, recomputes master mask.
    - `removeChannelOverride(channel)` — removes the override, gives
      that channel back to the master.
  - Auto-applies tempo from MIDI on upload.
- `protocol.ts` + `@moonlitt/protocol`: new commands `default.set_instrument`,
  `channel.set_override`, `channel.remove_override`. New events
  `default.instrument_changed`, `channel.override_added`,
  `channel.override_removed`, `transport.tempo_changed`.

**Web (TypeScript / React):**
- New `DefaultInstrumentBar` between MIDI bar and channel list.
- `TrackCard` becomes `ChannelRow` — represents one MIDI channel:
  - Shows channel number + name from MIDI metadata.
  - Inherits-default state vs override state.
  - "Override instrument…" button (becomes "Change… [×]" once set).
  - M/S/volume/effects (effects only meaningful when there's an override
    — disable for inherited channels, since the master's effects chain
    is shared across all inherited channels).
- New `useDefaultInstrumentStore` (or fold into mixer store).

## Mute / solo semantics

For an inherited channel, M/S routes by setting that channel's bit in a
"mute mask" / "solo mask" applied at mixer dispatch time — needs a new
small mixer op `set_channel_mute(channel, muted)`. For an overridden
channel, M/S applies to the override track (existing path).

This is the only piece that needs a real engine extension. Until it
ships, M/S on inherited channels is hidden (only override rows show
M/S).

## What gets removed

- `loadMidiMultitrack`'s "one DAW track per channel" auto-create.
- Per-track MIDI clip metadata (the channel row replaces it — there's
  one MIDI for the whole project, shown once at the top).

## What stays the same

- The two pain points already fixed: SF2 scanner with symlink support,
  global MIDI upload bar.
- Insert chain on override tracks (existing UI re-used).
- Plugin scan / picker modal.

## Testing

Manual end-to-end:

1. Drop `examples/midi-test/Prelude1.mid` (1 channel) →
   - Master picker prompts for an SF2 if none set.
   - Pick `GeneralUser_GS` → press Play → audible piano.
2. Drop a GM file like `voyage.mid` (3 channels) →
   - Channel rows appear with names from the MIDI.
   - Press Play → all 3 channels play through GeneralUser_GS, each
     using the GM patch the file specifies.
3. Click "Override" on a channel → pick a different SF2 →
   - That channel switches; the others keep the default.
4. Change BPM in transport bar → playback speed changes.

If step 1/2 doesn't make sound, the play snapshot log already added in
the previous round will show whether the master has a backend and what
its mask is — same diagnostic surface.

## Localisation

UI is single-user, single-language — Chinese. Hardcode Chinese strings,
no i18n framework. Concrete copy:

- Header transport: `▶ 播放 / ❚❚ 暂停 / ■ 停止`, `位置 / 节拍`
- MIDI bar empty: `拖一个 .mid 文件到这里开始`
- MIDI bar loaded: `MIDI: <name> · <N> 个通道 · <bpm> BPM · <num/den> · <bars> 小节 · [更换…]`
- Default instrument: `默认音色`, `更换…`, `每个未单独指定音色的通道都用它播放`
- Channel section heading: `通道 (来自 MIDI 文件)`
- Channel row inherited: `沿用默认音色`, `[单独设置音色…]`
- Channel row overridden: `单独设置: <name>`, `[更换…] [×恢复默认]`
- Effect chain: `效果器`, `+ 添加效果`, `× 移除`

GM Program → Chinese mapping for the 128 General-MIDI patches lives in
`packages/web/src/i18n/gm-programs.ts`. Channel rows show:

- If MIDI's TrackName for this channel is set → show that as the row title.
- Else if the channel has a Program Change → look up GM Chinese name (e.g.
  program 0 → "大钢琴", program 25 → "尼龙弦吉他", program 128 → "鼓组"
  for channel 10).
- Else → "通道 N".

SF2 / VST3 / CLAP file names stay as-is (proper names).

## Out of scope (explicit)

- Per-channel program-change override (use the SF2's bank — the MIDI's
  PCs handle this for free).
- Drum-kit auto-detection for channel 10 (GM convention covers it).
- Saving / loading projects.
- Multiple MIDIs at once.
