using System;
using System.Runtime.InteropServices;
using System.Text.Json;

namespace Moonlitt;

/// <summary>
/// High-level wrapper around the moonlitt audio engine.
/// Supports loading plugins (VST3/CLAP/SF2), sending MIDI events,
/// and offline rendering to float buffers.
/// </summary>
public sealed class Engine : IDisposable
{
    private IntPtr _handle;
    private bool _disposed;

    /// <summary>
    /// Create a new audio engine.
    /// </summary>
    /// <param name="sampleRate">Audio sample rate (default 44100).</param>
    /// <param name="bufferSize">Internal buffer size in frames (default 256).</param>
    public Engine(int sampleRate = 44100, int bufferSize = 256)
    {
        NativeLibLoader.EnsureLoaded();
        _handle = NativeApi.moonlitt_engine_create(sampleRate, bufferSize);
        if (_handle == IntPtr.Zero)
            throw new InvalidOperationException("Failed to create engine");
    }

    /// <summary>Internal constructor used when wrapping an existing native handle.</summary>
    internal Engine(IntPtr handle)
    {
        _handle = handle;
    }

    internal IntPtr Handle => _handle;

    private void ThrowIfDisposed()
    {
        if (_disposed) throw new ObjectDisposedException(nameof(Engine));
    }

    /// <summary>
    /// Detach the native handle (ownership transferred to Runtime).
    /// After this call the Engine instance must not be used.
    /// </summary>
    internal void Detach()
    {
        _handle = IntPtr.Zero;
        _disposed = true;
        GC.SuppressFinalize(this);
    }

    /// <summary>Whether a plugin/soundfont backend is currently loaded.</summary>
    public bool IsLoaded
    {
        get
        {
            ThrowIfDisposed();
            return NativeApi.moonlitt_engine_is_loaded(_handle) != 0;
        }
    }

    // -------------------------------------------------------------------
    // Loading
    // -------------------------------------------------------------------

    /// <summary>
    /// Load a plugin or soundfont. Auto-detects format by file extension.
    /// </summary>
    /// <exception cref="MoonlittException">Thrown when loading fails.</exception>
    public void Load(string path)
    {
        ThrowIfDisposed();
        if (NativeApi.moonlitt_engine_load(_handle, path) != 0)
            throw new MoonlittException(GetError() ?? $"Failed to load: {path}");
    }

    /// <summary>Unload the current backend.</summary>
    public void Unload()
    {
        ThrowIfDisposed();
        NativeApi.moonlitt_engine_unload(_handle);
    }

    // -------------------------------------------------------------------
    // MIDI
    // -------------------------------------------------------------------

    public void NoteOn(int channel, int note, int velocity)
    {
        ThrowIfDisposed();
        NativeApi.moonlitt_engine_note_on(_handle, channel, note, velocity);
    }

    public void NoteOff(int channel, int note)
    {
        ThrowIfDisposed();
        NativeApi.moonlitt_engine_note_off(_handle, channel, note);
    }

    public void CC(int channel, int cc, int value)
    {
        ThrowIfDisposed();
        NativeApi.moonlitt_engine_cc(_handle, channel, cc, value);
    }

    public void PitchBend(int channel, int value)
    {
        ThrowIfDisposed();
        NativeApi.moonlitt_engine_pitch_bend(_handle, channel, value);
    }

    public void ProgramChange(int channel, int program)
    {
        ThrowIfDisposed();
        NativeApi.moonlitt_engine_program_change(_handle, channel, program);
    }

    public void AllNotesOff()
    {
        ThrowIfDisposed();
        NativeApi.moonlitt_engine_all_notes_off(_handle);
    }

    // -------------------------------------------------------------------
    // Render
    // -------------------------------------------------------------------

    /// <summary>
    /// Render audio into stereo float buffers.
    /// Both arrays must have the same length.
    /// </summary>
    public unsafe void Render(float[] left, float[] right)
    {
        ThrowIfDisposed();
        if (left.Length != right.Length)
            throw new ArgumentException("left and right buffers must have the same length");

        fixed (float* l = left, r = right)
            NativeApi.moonlitt_engine_render(_handle, l, r, left.Length);
    }

    /// <summary>Set the master volume (0.0 = silence, 1.0 = unity).</summary>
    public void SetVolume(float volume)
    {
        ThrowIfDisposed();
        NativeApi.moonlitt_engine_set_volume(_handle, volume);
    }

    // -------------------------------------------------------------------
    // Plugins / Presets
    // -------------------------------------------------------------------

    /// <summary>
    /// Scan for available audio plugins. Returns the raw JSON array string.
    /// </summary>
    public string? ScanPluginsJson()
    {
        ThrowIfDisposed();
        return MarshalFreeString(NativeApi.moonlitt_engine_scan_plugins(_handle));
    }

    /// <summary>
    /// Scan for available audio plugins, deserialized into an array.
    /// </summary>
    public PluginInfo[] ScanPlugins()
    {
        ThrowIfDisposed();
        var json = ScanPluginsJson();
        if (string.IsNullOrEmpty(json)) return Array.Empty<PluginInfo>();

        using var doc = JsonDocument.Parse(json);
        var root = doc.RootElement;
        var result = new PluginInfo[root.GetArrayLength()];
        for (var i = 0; i < result.Length; i++)
        {
            var el = root[i];
            result[i] = new PluginInfo(
                el.GetProperty("name").GetString() ?? "",
                el.GetProperty("path").GetString() ?? "",
                el.GetProperty("format").GetString() ?? "");
        }
        return result;
    }

    /// <summary>
    /// Get presets for the currently loaded backend. Returns the raw JSON array string.
    /// </summary>
    public string? GetPresetsJson()
    {
        ThrowIfDisposed();
        return MarshalFreeString(NativeApi.moonlitt_engine_get_presets(_handle));
    }

    /// <summary>
    /// Get presets for the currently loaded backend, deserialized into an array.
    /// </summary>
    public PresetInfo[] GetPresets()
    {
        ThrowIfDisposed();
        var json = GetPresetsJson();
        if (string.IsNullOrEmpty(json)) return Array.Empty<PresetInfo>();

        using var doc = JsonDocument.Parse(json);
        var root = doc.RootElement;
        var result = new PresetInfo[root.GetArrayLength()];
        for (var i = 0; i < result.Length; i++)
        {
            var el = root[i];
            result[i] = new PresetInfo(
                el.GetProperty("id").GetInt32(),
                el.GetProperty("name").GetString() ?? "");
        }
        return result;
    }

    /// <summary>
    /// Load a preset by ID.
    /// </summary>
    /// <exception cref="MoonlittException">Thrown when preset loading fails.</exception>
    public void LoadPreset(int id)
    {
        ThrowIfDisposed();
        if (NativeApi.moonlitt_engine_load_preset(_handle, id) != 0)
            throw new MoonlittException(GetError() ?? "Failed to load preset");
    }

    /// <summary>
    /// Detail of the most recent failure on the calling thread, or null.
    /// (Thread-local; only meaningful right after a failed call.)
    /// </summary>
    public static string? GetError()
    {
        var ptr = NativeApi.moonlitt_last_error_message();
        return ptr == IntPtr.Zero ? null : Marshal.PtrToStringUTF8(ptr);
    }

    // -------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------

    private static string? MarshalFreeString(IntPtr ptr)
    {
        if (ptr == IntPtr.Zero) return null;
        var s = Marshal.PtrToStringUTF8(ptr);
        NativeApi.moonlitt_free_string(ptr);
        return s;
    }

    // -------------------------------------------------------------------
    // IDisposable
    // -------------------------------------------------------------------

    public void Dispose()
    {
        if (_disposed) return;
        _disposed = true;
        if (_handle != IntPtr.Zero)
        {
            NativeApi.moonlitt_engine_destroy(_handle);
            _handle = IntPtr.Zero;
        }
        GC.SuppressFinalize(this);
    }

    ~Engine() => Dispose();
}
