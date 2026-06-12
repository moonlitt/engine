using System;
using System.IO;
using Xunit;

namespace Moonlitt.Tests;

public class RuntimeTests
{
    /// A Runtime needs a loaded backend — an empty engine is rejected by
    /// design (NOT_LOADED). These tests therefore need an SF2; they skip
    /// (return early) when none is available on the machine.
    private static Engine? CreateLoadedEngine()
    {
        var sf2 = Environment.GetEnvironmentVariable("MOONLITT_SF2")
            ?? "/Users/wangyan/Desktop/stardew valley mods/soundfonts/GeneralUser_GS.sf2";
        if (!File.Exists(sf2)) return null;
        var engine = new Engine(44100, 512);
        engine.Load(sf2);
        return engine;
    }

    [Fact]
    public void CreateFromEmptyEngineThrows()
    {
        var engine = new Engine(44100, 512);
        // No backend loaded — runtime creation must fail loudly, not UB.
        Assert.ThrowsAny<Exception>(() => new Runtime(engine));
        engine.Dispose();
    }

    [Fact]
    public void CreateAndDispose()
    {
        var engine = CreateLoadedEngine();
        if (engine == null) return; // no SF2 on this machine — skip
        using var runtime = new Runtime(engine);
        // Engine handle's backend was transferred — engine is now a shell.
    }

    [Fact]
    public void StartAndStop()
    {
        var engine = CreateLoadedEngine();
        if (engine == null) return;
        using var runtime = new Runtime(engine);

        runtime.Start();
        runtime.Stop();
    }

    [Fact]
    public void MidiMethodsDoNotThrow()
    {
        var engine = CreateLoadedEngine();
        if (engine == null) return;
        using var runtime = new Runtime(engine);
        runtime.Start();

        runtime.NoteOn(0, 60, 100);
        runtime.NoteOff(0, 60);
        runtime.CC(0, 64, 127);
        runtime.PitchBend(0, 0);
        runtime.AllNotesOff();
        runtime.SetVolume(0.8f);

        runtime.Stop();
    }

    [Fact]
    public void TransportMethodsDoNotThrow()
    {
        var engine = CreateLoadedEngine();
        if (engine == null) return;
        using var runtime = new Runtime(engine);

        runtime.Play();
        runtime.Pause();
        runtime.StopPlayback();
    }

    [Fact]
    public void ListMidiInputsReturnsArray()
    {
        var devices = Runtime.ListMidiInputs();
        Assert.NotNull(devices);
        // May be empty depending on system MIDI devices
    }

    [Fact]
    public void DoubleDisposeIsSafe()
    {
        var engine = CreateLoadedEngine();
        if (engine == null) return;
        var runtime = new Runtime(engine);
        runtime.Dispose();
        runtime.Dispose(); // should not throw
    }
}
