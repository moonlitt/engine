using System;
using System.Runtime.InteropServices;

namespace Moonlitt;

/// <summary>
/// MoonlittStatus codes (mirror of include/moonlitt.h).
/// 0 = success; negative values classify the failure.
/// </summary>
public static class MoonlittStatus
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

/// <summary>
/// Raw P/Invoke declarations for libmoonlitt (ABI draft 0.9).
/// All signatures match include/moonlitt.h.
/// </summary>
internal static class NativeApi
{
    private const string Lib = "moonlitt";

    // -----------------------------------------------------------------------
    // Library-wide
    // -----------------------------------------------------------------------

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern uint moonlitt_abi_version();

    /// Borrowed pointer (do NOT free): detail of the most recent failure
    /// on the calling thread, or NULL.
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr moonlitt_last_error_message();

    // -----------------------------------------------------------------------
    // Engine — lifecycle
    // -----------------------------------------------------------------------

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr moonlitt_engine_create(int sampleRate, int bufferSize);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void moonlitt_engine_destroy(IntPtr e);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int moonlitt_engine_load(
        IntPtr e,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string path);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void moonlitt_engine_unload(IntPtr e);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int moonlitt_engine_is_loaded(IntPtr e);

    // -----------------------------------------------------------------------
    // Engine — MIDI (status-returning)
    // -----------------------------------------------------------------------

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int moonlitt_engine_note_on(IntPtr e, int ch, int note, int vel);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int moonlitt_engine_note_off(IntPtr e, int ch, int note);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int moonlitt_engine_cc(IntPtr e, int ch, int cc, int val);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int moonlitt_engine_pitch_bend(IntPtr e, int ch, int val);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int moonlitt_engine_program_change(IntPtr e, int ch, int prog);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int moonlitt_engine_all_notes_off(IntPtr e);

    // -----------------------------------------------------------------------
    // Engine — render / volume
    // -----------------------------------------------------------------------

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern unsafe int moonlitt_engine_render(
        IntPtr e, float* left, float* right, int frames);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int moonlitt_engine_set_volume(IntPtr e, float volume);

    // -----------------------------------------------------------------------
    // Engine — plugins / presets
    // -----------------------------------------------------------------------

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr moonlitt_engine_scan_plugins(IntPtr e);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr moonlitt_engine_get_presets(IntPtr e);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int moonlitt_engine_load_preset(IntPtr e, int id);

    // -----------------------------------------------------------------------
    // Runtime — lifecycle + audio stream
    // -----------------------------------------------------------------------

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr moonlitt_runtime_create(IntPtr engine);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void moonlitt_runtime_destroy(IntPtr rt);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int moonlitt_runtime_start_audio(IntPtr rt);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int moonlitt_runtime_stop_audio(IntPtr rt);

    // -----------------------------------------------------------------------
    // Runtime — MIDI (lock-free SPSC; QueueFull when the ring is full)
    // -----------------------------------------------------------------------

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int moonlitt_runtime_note_on(IntPtr rt, int ch, int note, int vel);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int moonlitt_runtime_note_off(IntPtr rt, int ch, int note);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int moonlitt_runtime_cc(IntPtr rt, int ch, int cc, int val);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int moonlitt_runtime_pitch_bend(IntPtr rt, int ch, int val);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int moonlitt_runtime_all_notes_off(IntPtr rt);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int moonlitt_runtime_set_volume(IntPtr rt, float volume);

    // -----------------------------------------------------------------------
    // Runtime — MIDI devices
    // -----------------------------------------------------------------------

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr moonlitt_runtime_list_midi_inputs();

    // -----------------------------------------------------------------------
    // Runtime — transport
    // -----------------------------------------------------------------------

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int moonlitt_runtime_play(IntPtr rt);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int moonlitt_runtime_pause(IntPtr rt);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int moonlitt_runtime_stop(IntPtr rt);

    // -----------------------------------------------------------------------
    // Shared
    // -----------------------------------------------------------------------

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void moonlitt_free_string(IntPtr s);
}
