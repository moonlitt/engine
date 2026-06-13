using System;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using HarmonyLib;
using Moonlitt;
using StardewModdingAPI;
using StardewModdingAPI.Events;

namespace MoonlittPiano;

/// <summary>
/// The original moonlitt vision: blocks in a game making real music.
/// Flute and drum blocks route through the moonlitt engine — a GM
/// SoundFont out of the box, or a full .mlsession designed in the
/// moonlitt desktop app (Keyscape patch states included).
/// </summary>
public sealed class ModEntry : Mod
{
    internal static Runtime? Sound;
    internal static ModConfig Config = new();
    internal static IMonitor? Log;

    public override void Entry(IModHelper helper)
    {
        Log = Monitor;
        Config = helper.ReadConfig<ModConfig>();

        // Bind the engine BEFORE any P/Invoke fires. The binding's
        // DllImport name is "moonlitt"; resolve it to the dylib shipped
        // in the mod folder (default .NET probing never looks there).
        var dylib = Path.Combine(helper.DirectoryPath, "libmoonlitt.dylib");
        if (NativeLibrary.TryLoad(dylib, out var handle))
        {
            NativeLibrary.SetDllImportResolver(
                typeof(ModEntry).Assembly,
                (name, _, _) => name == "moonlitt" ? handle : IntPtr.Zero);
        }
        else
        {
            Monitor.Log($"engine library not found at {dylib} — mod stays silent", LogLevel.Error);
            return;
        }

        // Audio device init belongs after the game window exists.
        helper.Events.GameLoop.GameLaunched += (_, _) => InitAudio();
        helper.Events.GameLoop.ReturnedToTitle += (_, _) => Sound?.AllNotesOff();

        helper.ConsoleCommands.Add(
            "moonlitt_test",
            "Play a test arpeggio through the moonlitt engine.",
            (_, _) => TestArpeggio());

        var harmony = new Harmony(ModManifest.UniqueID);
        NoteBlockPatches.Apply(harmony, Monitor);
    }

    private void InitAudio()
    {
        try
        {
            Sound = CreateRuntime();
            if (Sound == null)
            {
                Monitor.Log(
                    "no sound source found — set SessionPath or Sf2Path in config.json, " +
                    "or drop a .sf2 into ~/Library/Audio/Sounds/Banks",
                    LogLevel.Warn);
                return;
            }
            Sound.Start();
            Sound.SetVolume(Math.Clamp(Config.Volume, 0f, 1f));
            Monitor.Log("moonlitt engine running — flute & drum blocks are live instruments now", LogLevel.Info);
        }
        catch (Exception ex)
        {
            // Never take the game down over audio.
            Sound = null;
            Monitor.Log($"moonlitt init failed: {ex.Message}", LogLevel.Error);
        }
    }

    private Runtime? CreateRuntime()
    {
        // Priority 1: a session designed in the moonlitt desktop app.
        if (!string.IsNullOrWhiteSpace(Config.SessionPath))
        {
            var reason = Runtime.ValidateSessionFile(Config.SessionPath);
            if (reason == null)
            {
                Monitor.Log($"loading session: {Config.SessionPath}", LogLevel.Info);
                return Runtime.LoadSession(Config.SessionPath);
            }
            Monitor.Log($"SessionPath unusable ({reason}) — falling back to SF2", LogLevel.Warn);
        }

        // Priority 2: a GM SoundFont (explicit path, else the standard dir).
        var sf2 = !string.IsNullOrWhiteSpace(Config.Sf2Path) ? Config.Sf2Path : FindSoundFont();
        if (sf2 == null) return null;
        Monitor.Log($"loading SoundFont: {sf2}", LogLevel.Info);
        return Runtime.CreateMultitrackSf2(sf2);
    }

    private static string? FindSoundFont()
    {
        var home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
        var banks = Path.Combine(home, "Library/Audio/Sounds/Banks");
        if (!Directory.Exists(banks)) return null;
        return Directory.EnumerateFiles(banks, "*.sf2")
            .OrderBy(p => Path.GetFileName(p).Contains("GeneralUser", StringComparison.OrdinalIgnoreCase) ? 0 : 1)
            .ThenBy(p => p)
            .FirstOrDefault();
    }

    private void TestArpeggio()
    {
        if (Sound == null)
        {
            Monitor.Log("engine not running", LogLevel.Warn);
            return;
        }
        int[] notes = { 60, 64, 67, 72 };
        for (var i = 0; i < notes.Length; i++)
        {
            // Sample-accurate scheduling: one call per note, no timers.
            Sound.NoteOnDelayed(0, notes[i], 100, i * 12_000);
            Sound.NoteOffDelayed(0, notes[i], i * 12_000 + 48_000);
        }
        Monitor.Log("arpeggio sent", LogLevel.Info);
    }
}
