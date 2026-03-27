using System;
using Xunit;

namespace Moonlitt.Tests;

public class RuntimeTests
{
    [Fact]
    public void CreateAndDispose()
    {
        var engine = new Engine(44100, 512);
        using var runtime = new Runtime(engine);
        // Engine handle was transferred — engine is now detached
    }

    [Fact]
    public void StartAndStop()
    {
        var engine = new Engine(44100, 512);
        using var runtime = new Runtime(engine);

        runtime.Start();
        runtime.Stop();
    }

    [Fact]
    public void MidiMethodsDoNotThrow()
    {
        var engine = new Engine(44100, 512);
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
        var engine = new Engine(44100, 512);
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
        var engine = new Engine(44100, 512);
        var runtime = new Runtime(engine);
        runtime.Dispose();
        runtime.Dispose(); // should not throw
    }
}
