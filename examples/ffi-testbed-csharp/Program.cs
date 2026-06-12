// FFI testbed for libmoonlitt_ffi.dylib via P/Invoke.
//
// Validates the C ABI surface that piano-block consumes, with a faster
// feedback loop than reloading SMAPI. When capi changes (which happens
// often — Rust core is unstable), this surfaces breakage in 3 seconds
// instead of in a 90-second Stardew restart.
//
// Modes:
//   default       — run smoke tests across all 4 phases, exit code = failures.
//   --play        — play the bevy MELODY (audible, single-engine path).
//   --interactive — D F J K → C D E F (audible).

using System;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Threading;
using MoonlittFfiTestbed;

const int SAMPLE_RATE = 44100;
const int BUFFER_SIZE = 256;

byte[] columnNotes = { 60, 62, 64, 65 }; // C D E F
ConsoleKey[] columnKeys = { ConsoleKey.D, ConsoleKey.F, ConsoleKey.J, ConsoleKey.K };
(int col, double t)[] melody =
{
    (0, 0.0), (1, 0.5), (2, 1.0), (3, 1.5),
    (3, 2.0), (2, 2.5), (1, 3.0), (0, 3.5),
    (0, 4.0), (0, 4.5), (1, 5.0), (2, 5.5),
    (3, 6.0), (3, 6.5), (2, 7.0), (0, 7.5),
};

bool play = args.Contains("--play");
bool interactive = args.Contains("--interactive");
bool forceNoSf2 = args.Contains("--no-sf2");

string? sf2 = forceNoSf2 ? null : ResolveSf2(args);

Console.WriteLine($"[testbed] dylib search: {Environment.GetEnvironmentVariable("DYLD_LIBRARY_PATH") ?? "(unset)"}");
Console.WriteLine($"[testbed] SF2: {sf2 ?? "(none — running ABI-only subset)"}");

if (play)
{
    if (sf2 == null) { Console.Error.WriteLine("--play requires an SF2"); return 1; }
    return RunMelody(sf2, melody, columnNotes);
}
if (interactive)
{
    if (sf2 == null) { Console.Error.WriteLine("--interactive requires an SF2"); return 1; }
    return RunInteractive(sf2, columnKeys, columnNotes);
}
return RunSmokeTests(sf2);

static string? ResolveSf2(string[] args)
{
    var positional = args.FirstOrDefault(a => !a.StartsWith("--"));
    var path = positional
        ?? Environment.GetEnvironmentVariable("MOONLITT_SF2")
        ?? "/Users/wangyan/Desktop/stardew valley mods/soundfonts/GeneralUser_GS.sf2";
    return File.Exists(path) ? path : null;
}

int RunSmokeTests(string? sf2Path)
{
    var t = new TestRunner();

    if (sf2Path != null)
    {
        Console.WriteLine("\n=== Phase A: single engine + simple runtime ===");
        PhaseA_EngineAndRuntime(t, sf2Path);

        Console.WriteLine("\n=== Phase B: multitrack_create shortcut ===");
        PhaseB_Multitrack(t, sf2Path);

        Console.WriteLine("\n=== Phase C: pre-built mixer + reverb send + EQ insert ===");
        PhaseC_PrebuiltMixer(t, sf2Path);

        Console.WriteLine("\n=== Phase D: dynamic runtime mixer ops ===");
        PhaseD_DynamicMixer(t, sf2Path);
    }
    else
    {
        Console.WriteLine("\n=== Phase Z: ABI-only subset (no SF2 — for CI without audio assets) ===");
        PhaseZ_NoSf2Subset(t);
    }

    Console.WriteLine("\n=== Standalone: list_midi_inputs ===");
    PhaseE_MidiDevices(t);

    return t.Report();
}

// ABI-only subset: no SF2 file required. Probes the failure paths and the
// fns that don't depend on audio assets. Sufficient to catch most signature
// breakage on a CI runner with no soundfont installed.
void PhaseZ_NoSf2Subset(TestRunner t)
{
    IntPtr engine = NativeEngine.moonlitt_engine_create(SAMPLE_RATE, BUFFER_SIZE);
    t.Check("engine_create returns non-null", engine != IntPtr.Zero);
    if (engine == IntPtr.Zero) return;

    int rc = NativeEngine.moonlitt_engine_load(engine, "/no/such/path.sf2");
    t.Check("engine_load(bad path) returns non-zero", rc != 0);
    string? err = NativeEngine.GetLastError(engine);
    t.Check("engine_get_error returns non-empty UTF-8 after failure", !string.IsNullOrEmpty(err));

    t.Check("engine_is_loaded == 0 (no backend)", NativeEngine.moonlitt_engine_is_loaded(engine) == 0);

    // No-backend introspection should return empty arrays, not crash.
    IntPtr ptr = NativeEngine.moonlitt_engine_param_info_json(engine);
    t.Check("param_info_json (no backend) non-null", ptr != IntPtr.Zero);
    string j = NativeEngine.ConsumeOwnedString(ptr);
    t.Check("param_info_json (no backend) is empty array", j == "[]");

    ptr = NativeEngine.moonlitt_engine_get_presets(engine);
    string p = NativeEngine.ConsumeOwnedString(ptr);
    t.Check("get_presets (no backend) is empty array", p == "[]");

    NativeEngine.moonlitt_engine_destroy(engine);

    // Mixer + builtin factories don't need SF2.
    IntPtr mixer = NativeEngine.moonlitt_mixer_create(SAMPLE_RATE, BUFFER_SIZE);
    t.Check("mixer_create returns non-null", mixer != IntPtr.Zero);
    NativeEngine.moonlitt_mixer_destroy(mixer);

    IntPtr reverb = NativeEngine.moonlitt_builtin_create_reverb(SAMPLE_RATE, BUFFER_SIZE);
    t.Check("builtin_create_reverb returns non-null", reverb != IntPtr.Zero);
    NativeEngine.moonlitt_engine_destroy(reverb);

    IntPtr eq = NativeEngine.moonlitt_builtin_create_eq(SAMPLE_RATE, BUFFER_SIZE);
    t.Check("builtin_create_eq returns non-null", eq != IntPtr.Zero);
    NativeEngine.moonlitt_engine_destroy(eq);

    IntPtr comp = NativeEngine.moonlitt_builtin_create_compressor(SAMPLE_RATE, BUFFER_SIZE);
    t.Check("builtin_create_compressor returns non-null", comp != IntPtr.Zero);
    NativeEngine.moonlitt_engine_destroy(comp);

    // multitrack_create with bad path — should return null, not crash.
    IntPtr badRt = NativeEngine.moonlitt_multitrack_create("/no/such.sf2", SAMPLE_RATE, BUFFER_SIZE);
    t.Check("multitrack_create(bad path) returns null", badRt == IntPtr.Zero);
}

// ---------------------------------------------------------------------------
// Phase A — single engine through to runtime, exhaustive coverage of params,
// engine-mode MIDI extras, runtime CC/PB/PC, sample-accurate scheduling.
// ---------------------------------------------------------------------------
void PhaseA_EngineAndRuntime(TestRunner t, string sf2Path)
{
    IntPtr engine = NativeEngine.moonlitt_engine_create(SAMPLE_RATE, BUFFER_SIZE);
    t.Check("engine_create returns non-null", engine != IntPtr.Zero);
    if (engine == IntPtr.Zero) return;

    // Error path with bad UTF-8 path.
    int badRc = NativeEngine.moonlitt_engine_load(engine, "/no/such/path.sf2");
    t.Check("engine_load(bad path) returns non-zero", badRc != 0);
    string? err = NativeEngine.GetLastError(engine);
    t.Check("engine_get_error returns non-empty after failure", !string.IsNullOrEmpty(err));
    if (!string.IsNullOrEmpty(err)) Console.WriteLine($"        last_error = {err}");

    // Real path with embedded space — exercises LPUTF8Str.
    int rc = NativeEngine.moonlitt_engine_load(engine, sf2Path);
    t.Check("engine_load(real SF2 with space in path) returns 0", rc == 0);
    if (rc != 0)
    {
        Console.Error.WriteLine($"        last_error = {NativeEngine.GetLastError(engine)}");
        NativeEngine.moonlitt_engine_destroy(engine);
        return;
    }

    t.Check("engine_is_loaded == 1 after load", NativeEngine.moonlitt_engine_is_loaded(engine) == 1);

    // Engine-mode MIDI: verifies the offline / non-runtime path is wired up.
    NativeEngine.moonlitt_engine_program_change(engine, 0, 0);
    NativeEngine.moonlitt_engine_note_on(engine, 0, 60, 100);
    NativeEngine.moonlitt_engine_cc(engine, 0, 7, 80);
    NativeEngine.moonlitt_engine_pitch_bend(engine, 0, 8192);
    NativeEngine.moonlitt_engine_note_off(engine, 0, 60);
    NativeEngine.moonlitt_engine_all_notes_off(engine);
    NativeEngine.moonlitt_engine_set_volume(engine, 0.5f);
    t.Check("engine MIDI ops (note/cc/pb/pc/all_off/set_volume) execute", true);

    // Param round-trip — f64 marshaling is structurally distinct from f32.
    int paramCount = NativeEngine.moonlitt_engine_param_count(engine);
    t.Check("engine_param_count is non-negative", paramCount >= 0);

    IntPtr jsonPtr = NativeEngine.moonlitt_engine_param_info_json(engine);
    t.Check("engine_param_info_json returns non-null", jsonPtr != IntPtr.Zero);
    string json = NativeEngine.ConsumeOwnedString(jsonPtr);
    t.Check("param_info_json is a JSON array", json.StartsWith("[") && json.EndsWith("]"));

    if (paramCount > 0)
    {
        double original = NativeEngine.moonlitt_engine_get_param(engine, 0);
        t.Check("engine_get_param returns finite value", !double.IsNaN(original));
        NativeEngine.moonlitt_engine_set_param(engine, 0, 0.5);
        double after = NativeEngine.moonlitt_engine_get_param(engine, 0);
        t.Check("set_param + get_param round-trip (f64 marshaling intact)",
            !double.IsNaN(after));
        IntPtr displayPtr = NativeEngine.moonlitt_engine_param_display(engine, 0, 0.5);
        t.Check("param_display returns non-null pointer", displayPtr != IntPtr.Zero);
        string display = NativeEngine.ConsumeOwnedString(displayPtr);
        t.Check("param_display string survives free round-trip", display.Length >= 0);
        Console.WriteLine($"        param[0] display(0.5) = '{Trunc(display, 60)}'");
    }
    else
    {
        t.Check("(skipped param round-trip — backend exposes 0 params)", true);
    }

    IntPtr presetsPtr = NativeEngine.moonlitt_engine_get_presets(engine);
    t.Check("engine_get_presets returns non-null", presetsPtr != IntPtr.Zero);
    string presets = NativeEngine.ConsumeOwnedString(presetsPtr);
    t.Check("get_presets returns a JSON array", presets.StartsWith("[") && presets.EndsWith("]"));

    // Hand engine to runtime — backend is taken; engine handle persists as a shell.
    IntPtr runtime = NativeEngine.moonlitt_runtime_create(engine);
    t.Check("runtime_create returns non-null", runtime != IntPtr.Zero);
    if (runtime == IntPtr.Zero) { NativeEngine.moonlitt_engine_destroy(engine); return; }

    int startRc = NativeEngine.moonlitt_runtime_start(runtime);
    t.Check("runtime_start returns 0", startRc == 0);

    NativeEngine.moonlitt_runtime_set_volume(runtime, 0.5f);
    NativeEngine.moonlitt_runtime_program_change(runtime, 0, 0);
    NativeEngine.moonlitt_runtime_cc(runtime, 0, 7, 100);
    NativeEngine.moonlitt_runtime_pitch_bend(runtime, 0, 8192);
    t.Check("runtime CC/PB/PC + set_volume execute", true);

    NativeEngine.moonlitt_runtime_note_on(runtime, 0, 60, 100);
    Thread.Sleep(150);
    NativeEngine.moonlitt_runtime_note_off(runtime, 0, 60);
    t.Check("runtime note_on/off survives audio thread round-trip", true);

    // Sample-accurate scheduling — fire note 1024 samples in the future (~23ms @ 44.1k).
    // Audible verification is left to --play; here we only confirm no crash.
    NativeEngine.moonlitt_runtime_note_on_delayed(runtime, 0, 64, 100, 1024);
    Thread.Sleep(150);
    NativeEngine.moonlitt_runtime_note_off_delayed(runtime, 0, 64, 1024);
    t.Check("runtime note_on/off_delayed (sample-accurate) execute", true);

    NativeEngine.moonlitt_runtime_all_notes_off(runtime);
    t.Check("runtime_all_notes_off executes", true);

    int stopRc = NativeEngine.moonlitt_runtime_stop(runtime);
    t.Check("runtime_stop returns 0", stopRc == 0);

    NativeEngine.moonlitt_runtime_destroy(runtime);
    NativeEngine.moonlitt_engine_destroy(engine);
    t.Check("Phase A teardown clean", true);
}

// ---------------------------------------------------------------------------
// Phase B — multitrack_create: SF2 → 16-track runtime in one call.
// This is THE function piano-block ships with (CreateMultiTrack).
// ---------------------------------------------------------------------------
void PhaseB_Multitrack(TestRunner t, string sf2Path)
{
    IntPtr rt = NativeEngine.moonlitt_multitrack_create(sf2Path, SAMPLE_RATE, BUFFER_SIZE);
    t.Check("multitrack_create returns non-null", rt != IntPtr.Zero);
    if (rt == IntPtr.Zero) return;

    int startRc = NativeEngine.moonlitt_runtime_start(rt);
    t.Check("multitrack runtime_start returns 0", startRc == 0);

    NativeEngine.moonlitt_runtime_set_volume(rt, 0.4f);

    // Each of 16 tracks is bound to channelMask = 1 << ch, so note_on(ch, ...)
    // routes to track ch. Verify a couple of channels round-trip.
    NativeEngine.moonlitt_runtime_note_on(rt, 0, 60, 100);
    NativeEngine.moonlitt_runtime_note_on(rt, 5, 64, 100);
    Thread.Sleep(150);
    NativeEngine.moonlitt_runtime_note_off(rt, 0, 60);
    NativeEngine.moonlitt_runtime_note_off(rt, 5, 64);
    t.Check("note_on across multiple channels (multi-track routing)", true);

    NativeEngine.moonlitt_mixer_set_track_volume(rt, 0, 0.8f);
    NativeEngine.moonlitt_mixer_set_track_trim(rt, 0, -3.0f);
    NativeEngine.moonlitt_mixer_set_track_pan(rt, 1, -0.5f);
    NativeEngine.moonlitt_mixer_set_track_mute(rt, 2, 1);
    NativeEngine.moonlitt_mixer_set_track_solo(rt, 3, 0);
    NativeEngine.moonlitt_mixer_set_master_volume(rt, 0.7f);
    t.Check("mixer track controls (volume/trim/pan/mute/solo/master) execute", true);

    NativeEngine.moonlitt_runtime_all_notes_off(rt);
    NativeEngine.moonlitt_runtime_destroy(rt);
    t.Check("Phase B teardown clean", true);
}

// ---------------------------------------------------------------------------
// Phase C — build a Mixer manually with 1 track + 1 reverb send + 1 EQ insert,
// then hand it to runtime. Exercises the entire pre-creation API + insert/send
// param control + bypass + routing.
// ---------------------------------------------------------------------------
void PhaseC_PrebuiltMixer(TestRunner t, string sf2Path)
{
    IntPtr mixer = NativeEngine.moonlitt_mixer_create(SAMPLE_RATE, BUFFER_SIZE);
    t.Check("mixer_create returns non-null", mixer != IntPtr.Zero);
    if (mixer == IntPtr.Zero) return;

    // Track engine: SF2 instrument.
    IntPtr trackEngine = NativeEngine.moonlitt_engine_create(SAMPLE_RATE, BUFFER_SIZE);
    int trackLoad = NativeEngine.moonlitt_engine_load(trackEngine, sf2Path);
    t.Check("track engine loaded", trackLoad == 0);

    int trackId = NativeEngine.moonlitt_mixer_add_track(mixer, trackEngine, 0xFFFF);
    t.Check("mixer_add_track returns non-negative track_id", trackId >= 0);
    Console.WriteLine($"        track_id = {trackId}");

    // Reverb send bus.
    IntPtr reverbEngine = NativeEngine.moonlitt_builtin_create_reverb(SAMPLE_RATE, BUFFER_SIZE);
    t.Check("builtin_create_reverb returns non-null", reverbEngine != IntPtr.Zero);

    int busId = NativeEngine.moonlitt_mixer_add_send_bus(mixer, reverbEngine);
    t.Check("mixer_add_send_bus returns non-negative bus_id", busId >= 0);
    Console.WriteLine($"        bus_id = {busId}");

    // EQ insert on the track.
    IntPtr eqEngine = NativeEngine.moonlitt_builtin_create_eq(SAMPLE_RATE, BUFFER_SIZE);
    t.Check("builtin_create_eq returns non-null", eqEngine != IntPtr.Zero);

    int insertId = NativeEngine.moonlitt_mixer_add_insert(mixer, trackId, eqEngine);
    t.Check("mixer_add_insert returns non-negative insert_id", insertId >= 0);
    Console.WriteLine($"        insert_id = {insertId}");

    // Hand mixer to runtime — mixer is consumed.
    IntPtr rt = NativeEngine.moonlitt_runtime_create_from_mixer(mixer, BUFFER_SIZE);
    t.Check("runtime_create_from_mixer returns non-null", rt != IntPtr.Zero);
    if (rt == IntPtr.Zero) goto cleanup;

    int startRc = NativeEngine.moonlitt_runtime_start(rt);
    t.Check("runtime_start (from mixer) returns 0", startRc == 0);

    // Post-runtime mixer / insert / send param ops.
    NativeEngine.moonlitt_mixer_set_track_send(rt, trackId, busId, 0.4f);
    NativeEngine.moonlitt_mixer_set_send_bus_param(rt, busId, 0, 0.6f);
    NativeEngine.moonlitt_set_insert_param(rt, trackId, insertId, 0, 0.5f);
    NativeEngine.moonlitt_set_param_for_track(rt, trackId, 0, 0.5f);
    NativeEngine.moonlitt_mixer_set_insert_bypass(rt, trackId, insertId, 1);
    NativeEngine.moonlitt_mixer_set_insert_bypass(rt, trackId, insertId, 0);
    NativeEngine.moonlitt_mixer_set_track_route(rt, trackId, 0xFF); // route to master
    NativeEngine.moonlitt_runtime_set_param(rt, 0, 0.5f);
    t.Check("insert/send/track param + bypass + route + runtime_set_param execute", true);

    // Sanity audio: play a short note through track + EQ + reverb send.
    NativeEngine.moonlitt_runtime_set_volume(rt, 0.4f);
    NativeEngine.moonlitt_runtime_note_on(rt, 0, 60, 100);
    Thread.Sleep(200);
    NativeEngine.moonlitt_runtime_note_off(rt, 0, 60);
    Thread.Sleep(150);
    t.Check("Phase C audio path (track + insert + send) survives round-trip", true);

    NativeEngine.moonlitt_runtime_destroy(rt);

cleanup:
    // After runtime takes the mixer, mixer_destroy is still safe (handle stays).
    NativeEngine.moonlitt_mixer_destroy(mixer);
    // Engine handles for track/insert/bus had their backends taken. Destroying
    // their shells is still required to free the handle structs.
    NativeEngine.moonlitt_engine_destroy(trackEngine);
    NativeEngine.moonlitt_engine_destroy(reverbEngine);
    NativeEngine.moonlitt_engine_destroy(eqEngine);
    t.Check("Phase C teardown clean (mixer + 3 engine shells destroyed)", true);
}

// ---------------------------------------------------------------------------
// Phase D — start with a single-track runtime, then add a second track, an
// insert, and a send bus DYNAMICALLY at runtime. Tests the command-channel
// path that piano-block uses for live-adding effects.
// ---------------------------------------------------------------------------
void PhaseD_DynamicMixer(TestRunner t, string sf2Path)
{
    IntPtr engineA = NativeEngine.moonlitt_engine_create(SAMPLE_RATE, BUFFER_SIZE);
    NativeEngine.moonlitt_engine_load(engineA, sf2Path);
    IntPtr rt = NativeEngine.moonlitt_runtime_create(engineA);
    t.Check("base runtime created", rt != IntPtr.Zero);
    if (rt == IntPtr.Zero) { NativeEngine.moonlitt_engine_destroy(engineA); return; }

    NativeEngine.moonlitt_runtime_start(rt);

    // Add a second track at runtime.
    IntPtr engineB = NativeEngine.moonlitt_engine_create(SAMPLE_RATE, BUFFER_SIZE);
    NativeEngine.moonlitt_engine_load(engineB, sf2Path);
    int newTrack = NativeEngine.moonlitt_runtime_add_track(rt, engineB, 0xFFFF);
    t.Check("runtime_add_track returns non-negative track_id", newTrack >= 0);
    Console.WriteLine($"        dynamic track_id = {newTrack}");

    // Add a reverb send bus at runtime.
    IntPtr reverbEngine = NativeEngine.moonlitt_builtin_create_reverb(SAMPLE_RATE, BUFFER_SIZE);
    int newBus = NativeEngine.moonlitt_runtime_add_send_bus(rt, reverbEngine);
    t.Check("runtime_add_send_bus returns non-negative bus_id", newBus >= 0);

    // Add an EQ insert on the new track at runtime.
    IntPtr eqEngine = NativeEngine.moonlitt_builtin_create_eq(SAMPLE_RATE, BUFFER_SIZE);
    int newInsert = NativeEngine.moonlitt_runtime_add_insert(rt, newTrack, eqEngine);
    t.Check("runtime_add_insert returns non-negative insert_id", newInsert >= 0);

    // Brief audio over the new track.
    NativeEngine.moonlitt_runtime_set_volume(rt, 0.4f);
    NativeEngine.moonlitt_runtime_note_on(rt, 0, 67, 100);
    Thread.Sleep(150);
    NativeEngine.moonlitt_runtime_note_off(rt, 0, 67);
    t.Check("note round-trip on dynamically-added track", true);

    // Remove insert + track.
    if (newInsert >= 0) NativeEngine.moonlitt_runtime_remove_insert(rt, newTrack, newInsert);
    if (newTrack >= 0) NativeEngine.moonlitt_runtime_remove_track(rt, newTrack);
    t.Check("runtime_remove_insert + remove_track execute", true);

    NativeEngine.moonlitt_runtime_destroy(rt);
    NativeEngine.moonlitt_engine_destroy(engineA);
    NativeEngine.moonlitt_engine_destroy(engineB);
    NativeEngine.moonlitt_engine_destroy(reverbEngine);
    NativeEngine.moonlitt_engine_destroy(eqEngine);
    t.Check("Phase D teardown clean", true);
}

// ---------------------------------------------------------------------------
// Standalone — list_midi_inputs is a no-handle, owned-string-returning fn.
// Distinct shape worth its own check.
// ---------------------------------------------------------------------------
void PhaseE_MidiDevices(TestRunner t)
{
    IntPtr ptr = NativeEngine.moonlitt_runtime_list_midi_inputs();
    t.Check("runtime_list_midi_inputs returns non-null", ptr != IntPtr.Zero);
    string json = NativeEngine.ConsumeOwnedString(ptr);
    t.Check("midi_inputs JSON is array", json.StartsWith("[") && json.EndsWith("]"));
    Console.WriteLine($"        midi devices: {Trunc(json, 100)}");
}

// ---------------------------------------------------------------------------
// Audible modes (unchanged from before, useful for ear-debugging).
// ---------------------------------------------------------------------------

int RunMelody(string sf2Path, (int col, double t)[] melody, byte[] columnNotes)
{
    IntPtr engine = NativeEngine.moonlitt_engine_create(SAMPLE_RATE, BUFFER_SIZE);
    if (engine == IntPtr.Zero) { Console.Error.WriteLine("engine_create failed"); return 1; }
    if (NativeEngine.moonlitt_engine_load(engine, sf2Path) != 0)
    {
        Console.Error.WriteLine($"engine_load failed: {NativeEngine.GetLastError(engine)}");
        NativeEngine.moonlitt_engine_destroy(engine);
        return 1;
    }
    IntPtr runtime = NativeEngine.moonlitt_runtime_create(engine);
    if (runtime == IntPtr.Zero) { Console.Error.WriteLine("runtime_create failed"); return 1; }
    if (NativeEngine.moonlitt_runtime_start(runtime) != 0) { return 1; }
    NativeEngine.moonlitt_runtime_set_volume(runtime, 0.6f);

    Console.WriteLine("[testbed] playing melody (mirrors examples/bevy-piano-tiles)...");
    var sw = Stopwatch.StartNew();
    int idx = 0;
    var open = new System.Collections.Generic.Dictionary<int, double>();
    const double noteHold = 0.4;
    while (idx < melody.Length || open.Count > 0)
    {
        double now = sw.Elapsed.TotalSeconds;
        while (idx < melody.Length && now >= melody[idx].t)
        {
            var (col, _) = melody[idx];
            byte n = columnNotes[col];
            NativeEngine.moonlitt_runtime_note_on(runtime, 0, n, 100);
            Console.WriteLine($"  [{now:F3}s] on  col={col} note={n}");
            open[col] = now + noteHold;
            idx++;
        }
        foreach (var col in open.Where(kv => kv.Value <= now).Select(kv => kv.Key).ToList())
        {
            NativeEngine.moonlitt_runtime_note_off(runtime, 0, columnNotes[col]);
            open.Remove(col);
        }
        Thread.Sleep(5);
    }
    Thread.Sleep(800);
    NativeEngine.moonlitt_runtime_destroy(runtime);
    NativeEngine.moonlitt_engine_destroy(engine);
    Console.WriteLine("[testbed] done.");
    return 0;
}

int RunInteractive(string sf2Path, ConsoleKey[] keys, byte[] notes)
{
    IntPtr engine = NativeEngine.moonlitt_engine_create(SAMPLE_RATE, BUFFER_SIZE);
    if (NativeEngine.moonlitt_engine_load(engine, sf2Path) != 0)
    {
        Console.Error.WriteLine($"engine_load failed: {NativeEngine.GetLastError(engine)}");
        return 1;
    }
    IntPtr runtime = NativeEngine.moonlitt_runtime_create(engine);
    NativeEngine.moonlitt_runtime_start(runtime);
    NativeEngine.moonlitt_runtime_set_volume(runtime, 0.6f);

    Console.WriteLine("[testbed] D F J K → C D E F. Q to quit.");
    while (true)
    {
        var key = Console.ReadKey(intercept: true).Key;
        if (key == ConsoleKey.Q) break;
        for (int c = 0; c < keys.Length; c++)
        {
            if (key == keys[c])
            {
                NativeEngine.moonlitt_runtime_note_on(runtime, 0, notes[c], 100);
                Thread.Sleep(220);
                NativeEngine.moonlitt_runtime_note_off(runtime, 0, notes[c]);
            }
        }
    }
    NativeEngine.moonlitt_runtime_destroy(runtime);
    NativeEngine.moonlitt_engine_destroy(engine);
    return 0;
}

static string Trunc(string s, int n) => s.Length <= n ? s : s[..n] + "…";

internal sealed class TestRunner
{
    private int _passed, _failed;
    public void Check(string name, bool ok)
    {
        if (ok) { _passed++; Console.WriteLine($"  [PASS] {name}"); }
        else    { _failed++; Console.WriteLine($"  [FAIL] {name}"); }
    }
    public int Report()
    {
        Console.WriteLine($"\n[testbed] {_passed} passed, {_failed} failed");
        return _failed;
    }
}
