// P/Invoke declarations for libmoonlitt_ffi.dylib.
//
// These signatures are intentionally a verbatim subset of the ones piano-block
// ships in NativeEngine.cs — if this testbed loads and runs, the shared
// declarations are confirmed working end-to-end. Any signature drift between
// this file and piano-block's NativeEngine.cs is a bug.

using System;
using System.Runtime.InteropServices;

namespace MoonlittFfiTestbed;

internal static class NativeEngine
{
    private const string Lib = "moonlitt_ffi";

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

    // Returned pointer is owned by the engine handle — do NOT free.
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_engine_get_error(IntPtr e);

    // --- Engine MIDI (offline / pre-runtime mode) ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_engine_note_on(IntPtr e, int ch, int note, int vel);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_engine_note_off(IntPtr e, int ch, int note);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_engine_cc(IntPtr e, int ch, int cc, int val);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_engine_pitch_bend(IntPtr e, int ch, int val);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_engine_program_change(IntPtr e, int ch, int prog);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_engine_all_notes_off(IntPtr e);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_engine_set_volume(IntPtr e, float volume);

    // --- Engine params (note: f64, not f32 — distinct marshaling shape) ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_engine_param_count(IntPtr e);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_engine_param_info_json(IntPtr e);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern double moonlitt_engine_get_param(IntPtr e, int id);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_engine_set_param(IntPtr e, int id, double value);

    // Returns owned string — caller must free.
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_engine_param_display(IntPtr e, int id, double value);

    // --- Engine presets (returns owned string) ---
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
    public static extern int moonlitt_runtime_start(IntPtr rt);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_stop(IntPtr rt);

    // --- Runtime MIDI ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_runtime_note_on(IntPtr rt, int ch, int note, int vel);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_runtime_note_on_delayed(IntPtr rt, int ch, int note, int vel, int delaySamples);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_runtime_note_off(IntPtr rt, int ch, int note);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_runtime_note_off_delayed(IntPtr rt, int ch, int note, int delaySamples);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_runtime_cc(IntPtr rt, int ch, int cc, int val);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_runtime_pitch_bend(IntPtr rt, int ch, int val);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_runtime_program_change(IntPtr rt, int ch, int prog);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_runtime_all_notes_off(IntPtr rt);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_runtime_set_volume(IntPtr rt, float volume);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_runtime_set_param(IntPtr rt, int id, float value);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_runtime_list_midi_inputs();

    // --- Mixer pre-creation ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_mixer_create(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_mixer_destroy(IntPtr mixer);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_mixer_add_track(IntPtr mixer, IntPtr engineHandle, int channelMask);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_mixer_add_send_bus(IntPtr mixer, IntPtr engineHandle);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_mixer_add_insert(IntPtr mixer, int trackId, IntPtr engineHandle);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_runtime_create_from_mixer(IntPtr mixer, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_multitrack_create(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string sf2Path, int sampleRate, int bufferSize);

    // --- Built-in effect factories ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_builtin_create_eq(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_builtin_create_compressor(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr moonlitt_builtin_create_reverb(int sampleRate, int bufferSize);

    // --- Mixer track controls (require a runtime handle) ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_mixer_set_track_volume(IntPtr rt, int trackId, float vol);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_mixer_set_track_trim(IntPtr rt, int trackId, float trimDb);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_mixer_set_track_pan(IntPtr rt, int trackId, float pan);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_mixer_set_track_mute(IntPtr rt, int trackId, int mute);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_mixer_set_track_solo(IntPtr rt, int trackId, int solo);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_mixer_set_track_send(IntPtr rt, int trackId, int busId, float level);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_mixer_set_master_volume(IntPtr rt, float vol);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_mixer_set_track_route(IntPtr rt, int trackId, int targetId);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_mixer_set_send_bus_param(IntPtr rt, int busId, int paramId, float value);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_mixer_set_insert_bypass(IntPtr rt, int trackId, int insertId, int bypass);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_set_param_for_track(IntPtr rt, int trackId, int paramId, float value);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_set_insert_param(IntPtr rt, int trackId, int insertId, int paramId, float value);

    // --- Dynamic mixer ops on a live runtime ---
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_add_track(IntPtr rt, IntPtr engineHandle, int channelMask);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_runtime_remove_track(IntPtr rt, int trackId);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_add_insert(IntPtr rt, int trackId, IntPtr engineHandle);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void moonlitt_runtime_remove_insert(IntPtr rt, int trackId, int insertId);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int moonlitt_runtime_add_send_bus(IntPtr rt, IntPtr engineHandle);

    // --- Helpers (managed wrappers) ---

    public static string? GetLastError(IntPtr e)
    {
        var ptr = moonlitt_engine_get_error(e);
        return ptr == IntPtr.Zero ? null : Marshal.PtrToStringUTF8(ptr);
    }

    public static string ConsumeOwnedString(IntPtr ptr)
    {
        if (ptr == IntPtr.Zero) return string.Empty;
        try { return Marshal.PtrToStringUTF8(ptr) ?? string.Empty; }
        finally { moonlitt_free_string(ptr); }
    }
}
