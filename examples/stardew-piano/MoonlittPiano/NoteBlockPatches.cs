using System;
using HarmonyLib;
using StardewModdingAPI;
using StardewValley;
using StardewValley.Locations;
using SObject = StardewValley.Object;

namespace MoonlittPiano;

/// <summary>
/// Reroutes the two vanilla note blocks through moonlitt:
///
///   - Flute Block (O)464 — pitch lives in `preservedParentSheetIndex`
///     (0–2300 in steps of 100 = semitones; the right-click cycle
///     passes through a vanilla 2400 quirk). Mapped to
///     `FluteBaseNote + pitch/100` on the configured channel.
///   - Drum Block (O)463 — index 0–6 mapped onto GM percussion
///     (channel 9).
///
/// Each patch replicates the vanilla decompile's gating (1 s cooldown,
/// no dialogue, non-diagonal) and side effects (shake, scale pop, and
/// crucially `IslandSouthEast.OnFlutePlayed` — the island flute puzzle
/// must keep working), then skips the vanilla sound. When the engine
/// isn't running every patch falls through to vanilla untouched.
/// </summary>
internal static class NoteBlockPatches
{
    /// <summary>GM percussion for drumkit0..6 — kick, snare, closed
    /// hat, open hat, low tom, crash, clap (matching the vanilla kit's
    /// vibe rather than its exact samples).</summary>
    private static readonly int[] DrumNotes = { 36, 38, 42, 46, 45, 49, 39 };

    public static void Apply(Harmony harmony, IMonitor monitor)
    {
        try
        {
            harmony.Patch(
                original: AccessTools.Method(typeof(SObject), nameof(SObject.farmerAdjacentAction)),
                prefix: new HarmonyMethod(typeof(NoteBlockPatches), nameof(FarmerAdjacent_Prefix)));
            harmony.Patch(
                original: AccessTools.Method(typeof(SObject), "CheckForActionOnFluteBlock"),
                prefix: new HarmonyMethod(typeof(NoteBlockPatches), nameof(FluteInteract_Prefix)));
            harmony.Patch(
                original: AccessTools.Method(typeof(SObject), "CheckForActionOnDrumBlock"),
                prefix: new HarmonyMethod(typeof(NoteBlockPatches), nameof(DrumInteract_Prefix)));
        }
        catch (Exception ex)
        {
            monitor.Log($"Harmony patching failed — note blocks stay vanilla: {ex}", LogLevel.Error);
        }
    }

    /// <summary>Walk-by trigger for both block types.</summary>
    private static bool FarmerAdjacent_Prefix(SObject __instance, Farmer who, bool diagonal)
    {
        var sound = ModEntry.Sound;
        if (sound == null) return true; // engine down → vanilla

        switch (__instance.QualifiedItemId)
        {
            case "(O)464": // Flute Block
            {
                if (diagonal || Game1.dialogueUp) return false;
                var now = (int)Game1.currentGameTime.TotalGameTime.TotalMilliseconds;
                if (now - __instance.lastNoteBlockSoundTime < 1000) return false;
                int.TryParse(__instance.preservedParentSheetIndex.Value, out var pitch);
                PlayFlute(sound, pitch);
                Pop(__instance, now);
                // The island flute puzzle listens for played pitches.
                if (__instance.Location is IslandSouthEast island)
                    island.OnFlutePlayed(pitch);
                return false;
            }
            case "(O)463": // Drum Block
            {
                if (diagonal || Game1.dialogueUp) return false;
                var now = (int)Game1.currentGameTime.TotalGameTime.TotalMilliseconds;
                if (now - __instance.lastNoteBlockSoundTime < 1000) return false;
                int.TryParse(__instance.preservedParentSheetIndex.Value, out var index);
                PlayDrum(sound, index);
                Pop(__instance, now);
                return false;
            }
            default:
                return true; // everything else (slime balls etc.) → vanilla
        }
    }

    /// <summary>Right-click on a flute block: cycle pitch, play the new note.</summary>
    private static bool FluteInteract_Prefix(
        SObject __instance, bool justCheckingForActivity, ref bool __result)
    {
        var sound = ModEntry.Sound;
        if (sound == null) return true;
        __result = true;
        if (justCheckingForActivity) return false;

        int.TryParse(__instance.preservedParentSheetIndex.Value, out var pitch);
        // Vanilla's exact cycle, 2400 quirk included.
        pitch = pitch switch
        {
            2300 => 2400,
            2400 => 0,
            _ => (pitch + 100) % 2400,
        };
        __instance.preservedParentSheetIndex.Value = pitch.ToString();
        PlayFlute(sound, pitch);
        Pop(__instance, (int)Game1.currentGameTime.TotalGameTime.TotalMilliseconds);
        return false;
    }

    /// <summary>Right-click on a drum block: cycle kit piece, play it.</summary>
    private static bool DrumInteract_Prefix(
        SObject __instance, bool justCheckingForActivity, ref bool __result)
    {
        var sound = ModEntry.Sound;
        if (sound == null) return true;
        __result = true;
        if (justCheckingForActivity) return false;

        int.TryParse(__instance.preservedParentSheetIndex.Value, out var index);
        index = (index + 1) % 7;
        __instance.preservedParentSheetIndex.Value = index.ToString();
        PlayDrum(sound, index);
        Pop(__instance, (int)Game1.currentGameTime.TotalGameTime.TotalMilliseconds);
        return false;
    }

    private static void PlayFlute(Moonlitt.Runtime sound, int pitch)
    {
        var note = Math.Clamp(ModEntry.Config.FluteBaseNote + pitch / 100, 0, 127);
        var ch = Math.Clamp(ModEntry.Config.FluteChannel, 0, 15);
        sound.NoteOn(ch, note, 100);
        sound.NoteOffDelayed(ch, note, NoteLengthSamples());
    }

    private static void PlayDrum(Moonlitt.Runtime sound, int index)
    {
        var note = DrumNotes[Math.Clamp(index, 0, DrumNotes.Length - 1)];
        sound.NoteOn(9, note, 110);
        sound.NoteOffDelayed(9, note, NoteLengthSamples());
    }

    private static int NoteLengthSamples() =>
        (int)(Math.Clamp(ModEntry.Config.NoteSeconds, 0.1f, 10f) * 48_000);

    /// <summary>Vanilla's visual feedback: shake + vertical scale pop.</summary>
    private static void Pop(SObject obj, int nowMs)
    {
        obj.scale.Y = 1.3f;
        obj.shakeTimer = 200;
        obj.lastNoteBlockSoundTime = nowMs;
    }
}
