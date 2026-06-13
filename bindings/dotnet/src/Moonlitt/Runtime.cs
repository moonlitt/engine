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

    private Runtime(IntPtr handle)
    {
        _handle = handle;
    }

    /// <summary>
    /// Build a Runtime from a .mlsession file designed in the moonlitt
    /// desktop app — instruments, captured plug-in states (Keyscape
    /// patches included), mixer and send buses come up exactly as
    /// saved. Pre-started: call <see cref="Start"/> for sound.
    /// </summary>
    /// <exception cref="MoonlittException">Session missing/invalid or a referenced file is gone.</exception>
    public static Runtime LoadSession(string path, uint bufferSize = 256)
    {
        NativeLibLoader.EnsureLoaded();
        var handle = NativeApi.moonlitt_session_load_from_file(path, bufferSize);
        if (handle == IntPtr.Zero)
            throw new MoonlittException(Engine.GetError() ?? $"Failed to load session: {path}");
        return new Runtime(handle);
    }

    /// <summary>
    /// Pre-flight a session file: parses, schema-checks, and verifies
    /// every referenced file (plug-in, soundfont, MIDI clip) exists.
    /// Returns the failure reason, or null when the file is loadable.
    /// </summary>
    public static string? ValidateSessionFile(string path)
    {
        NativeLibLoader.EnsureLoaded();
        if (NativeApi.moonlitt_session_validate_file(path) == MoonlittStatus.Ok) return null;
        return Engine.GetError() ?? "session validation failed";
    }

    /// <summary>
    /// Build a 16-channel GM runtime from a SoundFont — each MIDI
    /// channel honours its own Program Change, so channel 9 is drums
    /// and melodic channels pick their own instruments. Pre-started:
    /// call <see cref="Start"/> for sound.
    /// </summary>
    /// <exception cref="MoonlittException">SoundFont missing or unparseable.</exception>
    public static Runtime CreateMultitrackSf2(string sf2Path, int sampleRate = 48_000, int bufferSize = 256)
    {
        NativeLibLoader.EnsureLoaded();
        var handle = NativeApi.moonlitt_runtime_create_multitrack_sf2(sf2Path, sampleRate, bufferSize);
        if (handle == IntPtr.Zero)
            throw new MoonlittException(Engine.GetError() ?? $"Failed to create SF2 runtime: {sf2Path}");
        return new Runtime(handle);
    }

    // -------------------------------------------------------------------
    // Audio output
    // -------------------------------------------------------------------

    /// <summary>Start the audio output stream.</summary>
    /// <exception cref="MoonlittException">Thrown when audio output fails to start.</exception>
    public void Start()
    {
        ThrowIfDisposed();
        if (NativeApi.moonlitt_runtime_start_audio(_handle) != 0)
            throw new MoonlittException(Engine.GetError() ?? "Failed to start audio output");
    }

    /// <summary>Stop the audio output stream.</summary>
    public void Stop()
    {
        ThrowIfDisposed();
        if (NativeApi.moonlitt_runtime_stop_audio(_handle) != 0)
            throw new MoonlittException(Engine.GetError() ?? "Failed to stop audio output");
    }

    // -------------------------------------------------------------------
    // MIDI (lock-free SPSC ring buffer — single caller only.
    // Do NOT call these methods from multiple threads concurrently.
    // The audio thread is the consumer; the caller is the sole producer.)
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

    /// <summary>Strike a note after <paramref name="delaySamples"/> frames (sample-accurate scheduling).</summary>
    public void NoteOnDelayed(int channel, int note, int velocity, int delaySamples)
    {
        ThrowIfDisposed();
        NativeApi.moonlitt_runtime_note_on_delayed(_handle, channel, note, velocity, delaySamples);
    }

    /// <summary>
    /// Release a note after <paramref name="delaySamples"/> frames —
    /// lets fire-and-forget callers schedule the whole envelope in one
    /// shot (e.g. game events with no natural note-off moment).
    /// </summary>
    public void NoteOffDelayed(int channel, int note, int delaySamples)
    {
        ThrowIfDisposed();
        NativeApi.moonlitt_runtime_note_off_delayed(_handle, channel, note, delaySamples);
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
        NativeApi.moonlitt_runtime_stop(_handle);
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
