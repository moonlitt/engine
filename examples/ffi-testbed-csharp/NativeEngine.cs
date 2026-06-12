// P/Invoke declarations for libmoonlitt.dylib (ABI draft 0.9).
//
// These signatures are intentionally the reference declarations that
// game mods (piano-block) copy verbatim — if this testbed loads and
// runs, the shared declarations are confirmed working end-to-end. Any
// signature drift between this file and include/moonlitt.h is a bug.
//
// Conventions:
//   * fallible functions return MoonlittStatus (0 = OK, negative = error
//     class); the detail string comes from moonlitt_last_error_message()
//     on the calling thread (borrowed — do NOT free).
//   * functions returning strings give OWNED pointers — free with
//     moonlitt_free_string.
//   * out-of-range arguments are rejected (INVALID_ARG), never clamped.

using System;
using System.Runtime.InteropServices;

namespace MoonlittFfiTestbed;

/// Mirror of the MoonlittStatus codes in include/moonlitt.h.
internal static class Status
{
    public const int Ok = 0;
    public const int InvalidArg = -1;
    public const int NotLoaded = -2;
    public const int QueueFull = -3;
    public const int Io = -4;
    public const int Plugin = -5;
    public const int State = -6;
    public const int Panic = -7;
    public const int Unsupported = -8;
}

internal static class NativeEngine
{
    private const string Lib = "moonlitt";

    // --- Library-wide: ABI version, error detail, panic-guard probe ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern uint moonlitt_abi_version();

    // Borrowed pointer — do NOT free. Detail of the most recent failure
    // on the calling thread.
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_last_error_message();

    // Deliberately panics inside the library; must return Status.Panic,
    // never crash the process. Proves the FFI panic guard through P/Invoke.
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_debug_trigger_panic();

    // --- Engine lifecycle ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_engine_create(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_engine_destroy(IntPtr e);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_engine_load(
        IntPtr e,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string path);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_engine_unload(IntPtr e);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_engine_is_loaded(IntPtr e);

    // --- Engine MIDI (offline / pre-runtime mode) ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_engine_note_on(IntPtr e, int ch, int note, int vel);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_engine_note_off(IntPtr e, int ch, int note);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_engine_cc(IntPtr e, int ch, int cc, int val);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_engine_pitch_bend(IntPtr e, int ch, int val);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_engine_program_change(IntPtr e, int ch, int prog);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_engine_all_notes_off(IntPtr e);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_engine_set_volume(IntPtr e, float volume);

    // --- Engine params (f64 marshaling) ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_engine_param_count(IntPtr e);

    // Owned string (or NULL when nothing is loaded) — free via helper.
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_engine_param_info_json(IntPtr e);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern double moonlitt_engine_get_param(IntPtr e, int id);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_engine_set_param(IntPtr e, int id, double value);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_engine_param_display(IntPtr e, int id, double value);

    // --- Engine presets (owned string, NULL when nothing is loaded) ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_engine_get_presets(IntPtr e);

    // --- moonlitt_free_string for any owned-string return ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_free_string(IntPtr s);

    // --- Runtime lifecycle ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_runtime_create(IntPtr engineHandle);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_runtime_destroy(IntPtr rt);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_start_audio(IntPtr rt);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_stop_audio(IntPtr rt);

    // --- Queries (atomic reads) ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_is_running(IntPtr rt);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_master_peak(IntPtr rt, out float left, out float right);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_master_rms(IntPtr rt, out float left, out float right);

    // --- Runtime MIDI (QueueFull when the SPSC ring is full) ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_note_on(IntPtr rt, int ch, int note, int vel);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_note_on_delayed(IntPtr rt, int ch, int note, int vel, int delaySamples);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_note_off(IntPtr rt, int ch, int note);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_note_off_delayed(IntPtr rt, int ch, int note, int delaySamples);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_cc(IntPtr rt, int ch, int cc, int val);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_pitch_bend(IntPtr rt, int ch, int val);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_program_change(IntPtr rt, int ch, int prog);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_all_notes_off(IntPtr rt);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_set_volume(IntPtr rt, float volume);

    // Backend params are f64 end-to-end.
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_set_param(IntPtr rt, int id, double value);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_runtime_list_midi_inputs();

    // --- Transport (sequencer) ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_play(IntPtr rt);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_pause(IntPtr rt);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_stop(IntPtr rt);

    // --- Mixer pre-creation ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_mixer_create(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_mixer_destroy(IntPtr mixer);

    // Non-negative id on success, negative MoonlittStatus on failure.
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_mixer_add_track(IntPtr mixer, IntPtr engineHandle, int channelMask);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_mixer_add_send_bus(IntPtr mixer, IntPtr engineHandle);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_mixer_add_insert(IntPtr mixer, int trackId, IntPtr engineHandle);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_runtime_create_from_mixer(IntPtr mixer, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_runtime_create_multitrack_sf2(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string sf2Path, int sampleRate, int bufferSize);

    // --- Built-in effect factories (all 19) ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_builtin_create_eq(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_builtin_create_compressor(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_builtin_create_reverb(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_builtin_create_limiter(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_builtin_create_gate(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_builtin_create_deesser(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_builtin_create_stereo_delay(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_builtin_create_chorus(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_builtin_create_flanger(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_builtin_create_phaser(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_builtin_create_tremolo(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_builtin_create_gain(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_builtin_create_stereo_width(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_builtin_create_saturator(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_builtin_create_bitcrusher(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_builtin_create_multiband_compressor(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_builtin_create_auto_filter(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_builtin_create_pitch_shifter(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_builtin_create_convolver(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string irPath, int sampleRate, int bufferSize);

    // --- Engine offline surface ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_engine_render(IntPtr e, [In, Out] float[] left, [In, Out] float[] right, int frames);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_engine_load_preset(IntPtr e, int id);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern float moonlitt_engine_measure_rms(IntPtr e, int program, int note, int velocity, int durationMs);

    // Owned string — free via helper.
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_engine_scan_plugins(IntPtr e);

    // --- Runtime mixer controls (renamed family: prefix == handle type) ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_set_track_volume(IntPtr rt, int trackId, float vol);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_set_track_trim(IntPtr rt, int trackId, float trimDb);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_set_track_pan(IntPtr rt, int trackId, float pan);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_set_track_mute(IntPtr rt, int trackId, int mute);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_set_track_solo(IntPtr rt, int trackId, int solo);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_set_track_send(IntPtr rt, int trackId, int busId, float level);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_set_master_volume(IntPtr rt, float vol);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_set_track_route(IntPtr rt, int trackId, int targetId);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_set_send_bus_param(IntPtr rt, int busId, int paramId, double value);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_set_insert_bypass(IntPtr rt, int trackId, int insertId, int bypass);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_set_track_param(IntPtr rt, int trackId, int paramId, double value);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_set_insert_param(IntPtr rt, int trackId, int insertId, int paramId, double value);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_set_insert_sidechain(IntPtr rt, int trackId, int insertId, int sourceTrackId);

    // --- Dynamic mixer ops on a live runtime ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_add_track(IntPtr rt, IntPtr engineHandle, int channelMask);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_remove_track(IntPtr rt, int trackId);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_add_insert(IntPtr rt, int trackId, IntPtr engineHandle);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_remove_insert(IntPtr rt, int trackId, int insertId);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_add_send_bus(IntPtr rt, IntPtr engineHandle);

    // --- Session files ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_session_load_from_file(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string path, uint bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_session_validate_file(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string path);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_session_read_json(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string path);

    // Declared-final signature; returns Status.Unsupported until ABI 1.0.
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_save_session(
        IntPtr rt, [MarshalAs(UnmanagedType.LPUTF8Str)] string path);


    // --- Patch state (capture once, replay headless) ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_engine_supports_state(IntPtr e);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_engine_save_state(IntPtr e, out IntPtr data, out nuint len);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_engine_load_state(IntPtr e, IntPtr data, nuint len);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_engine_recommended_warm_up_blocks(IntPtr e);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_engine_warm_up(IntPtr e, int blocks);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_free_buffer(IntPtr data, nuint len);

    // --- Helpers (managed wrappers) ---

    /// Thread-local detail of the most recent failure (borrowed — not freed).
    public static string? LastError()
    {
        var ptr = moonlitt_last_error_message();
        return ptr == IntPtr.Zero ? null : Marshal.PtrToStringUTF8(ptr);
    }

    public static string ConsumeOwnedString(IntPtr ptr)
    {
        if (ptr == IntPtr.Zero) return string.Empty;
        try { return Marshal.PtrToStringUTF8(ptr) ?? string.Empty; }
        finally { moonlitt_free_string(ptr); }
    }
}
