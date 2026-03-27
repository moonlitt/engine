using System;
using System.Runtime.InteropServices;
using System.Text.Json;

namespace Moonlitt;

/// <summary>
/// High-level wrapper around the moonlitt real-time audio runtime.
/// Takes ownership of an <see cref="Engine"/> and provides real-time
/// MIDI I/O and audio output via the system audio device.
/// </summary>
public sealed class Runtime : IDisposable
{
    private IntPtr _handle;
    private bool _disposed;

    private void ThrowIfDisposed()
    {
        if (_disposed) throw new ObjectDisposedException(nameof(Runtime));
    }

    /// <summary>
    /// Create a Runtime from an Engine.
    /// The Engine is consumed — ownership transfers to the native runtime.
    /// Do not use the Engine instance after this call.
    /// </summary>
    public Runtime(Engine engine)
    {
        NativeLibLoader.EnsureLoaded();
        _handle = NativeApi.moonlitt_runtime_create(engine.Handle);
        if (_handle == IntPtr.Zero)
            throw new InvalidOperationException("Failed to create runtime");
        engine.Detach(); // Engine is now owned by Runtime
    }

    // -------------------------------------------------------------------
    // Audio output
    // -------------------------------------------------------------------

    /// <summary>Start audio output.</summary>
    /// <exception cref="MoonlittException">Thrown when audio output fails to start.</exception>
    public void Start()
    {
        ThrowIfDisposed();
        if (NativeApi.moonlitt_runtime_start(_handle) != 0)
            throw new MoonlittException("Failed to start audio output");
    }

    /// <summary>Stop audio output.</summary>
    public void Stop()
    {
        ThrowIfDisposed();
        if (NativeApi.moonlitt_runtime_stop(_handle) != 0)
            throw new MoonlittException("Failed to stop audio output");
    }

    // -------------------------------------------------------------------
    // MIDI (thread-safe, lock-free via ring buffer)
    // -------------------------------------------------------------------

    public void NoteOn(int channel, int note, int velocity)
    {
        ThrowIfDisposed();
        NativeApi.moonlitt_runtime_note_on(_handle, channel, note, velocity);
    }

    public void NoteOff(int channel, int note)
    {
        ThrowIfDisposed();
        NativeApi.moonlitt_runtime_note_off(_handle, channel, note);
    }

    public void CC(int channel, int cc, int value)
    {
        ThrowIfDisposed();
        NativeApi.moonlitt_runtime_cc(_handle, channel, cc, value);
    }

    public void PitchBend(int channel, int value)
    {
        ThrowIfDisposed();
        NativeApi.moonlitt_runtime_pitch_bend(_handle, channel, value);
    }

    public void AllNotesOff()
    {
        ThrowIfDisposed();
        NativeApi.moonlitt_runtime_all_notes_off(_handle);
    }

    /// <summary>Set the master volume (0.0 = silence, 1.0 = unity).</summary>
    public void SetVolume(float volume)
    {
        ThrowIfDisposed();
        NativeApi.moonlitt_runtime_set_volume(_handle, volume);
    }

    // -------------------------------------------------------------------
    // MIDI devices
    // -------------------------------------------------------------------

    /// <summary>
    /// List available MIDI input devices. Returns the raw JSON array string.
    /// </summary>
    public static string? ListMidiInputsJson()
    {
        NativeLibLoader.EnsureLoaded();
        var ptr = NativeApi.moonlitt_runtime_list_midi_inputs();
        if (ptr == IntPtr.Zero) return null;
        var s = Marshal.PtrToStringUTF8(ptr);
        NativeApi.moonlitt_free_string(ptr);
        return s;
    }

    /// <summary>
    /// List available MIDI input devices, deserialized into an array.
    /// </summary>
    public static MidiDeviceInfo[] ListMidiInputs()
    {
        var json = ListMidiInputsJson();
        if (string.IsNullOrEmpty(json)) return Array.Empty<MidiDeviceInfo>();

        using var doc = JsonDocument.Parse(json);
        var root = doc.RootElement;
        var result = new MidiDeviceInfo[root.GetArrayLength()];
        for (var i = 0; i < result.Length; i++)
        {
            var el = root[i];
            result[i] = new MidiDeviceInfo(
                el.GetProperty("id").GetInt32(),
                el.GetProperty("name").GetString() ?? "");
        }
        return result;
    }

    // -------------------------------------------------------------------
    // Transport (sequencer control)
    // -------------------------------------------------------------------

    public void Play()
    {
        ThrowIfDisposed();
        NativeApi.moonlitt_runtime_play(_handle);
    }

    public void Pause()
    {
        ThrowIfDisposed();
        NativeApi.moonlitt_runtime_pause(_handle);
    }

    public void StopPlayback()
    {
        ThrowIfDisposed();
        NativeApi.moonlitt_runtime_stop_playback(_handle);
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
            NativeApi.moonlitt_runtime_destroy(_handle);
            _handle = IntPtr.Zero;
        }
        GC.SuppressFinalize(this);
    }

    ~Runtime() => Dispose();
}
