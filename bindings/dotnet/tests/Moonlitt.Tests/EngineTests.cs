using System;
using System.Threading;
using Xunit;

namespace Moonlitt.Tests;

public class EngineTests
{
    [Fact]
    public void CreateAndDispose()
    {
        using var engine = new Engine(44100, 256);
        Assert.False(engine.IsLoaded);
    }

    [Fact]
    public void RenderSilenceWhenNoBackendLoaded()
    {
        using var engine = new Engine(44100, 256);

        var left = new float[512];
        var right = new float[512];

        // Fill with non-zero to verify engine writes zeros
        Array.Fill(left, 1.0f);
        Array.Fill(right, 1.0f);

        engine.Render(left, right);

        // No backend loaded => should produce silence
        for (int i = 0; i < left.Length; i++)
        {
            Assert.Equal(0.0f, left[i]);
            Assert.Equal(0.0f, right[i]);
        }
    }

    [Fact]
    public void RenderThrowsOnMismatchedBufferLengths()
    {
        using var engine = new Engine();
        var left = new float[256];
        var right = new float[128];

        Assert.Throws<ArgumentException>(() => engine.Render(left, right));
    }

    [Fact]
    public void MidiMethodsDoNotThrowWithoutBackend()
    {
        using var engine = new Engine();

        // These should be no-ops when no backend is loaded (not throw)
        engine.NoteOn(0, 60, 100);
        engine.NoteOff(0, 60);
        engine.CC(0, 64, 127);
        engine.PitchBend(0, 0);
        engine.ProgramChange(0, 0);
        engine.AllNotesOff();
        engine.SetVolume(0.5f);
    }

    [Fact]
    public void LoadInvalidPathThrows()
    {
        using var engine = new Engine();
        Assert.Throws<MoonlittException>(() => engine.Load("/nonexistent/path.sf2"));
    }

    [Fact]
    public void UnloadIsIdempotent()
    {
        using var engine = new Engine();
        engine.Unload(); // no-op when nothing loaded
        engine.Unload(); // still no-op
    }

    [Fact]
    public void ScanPluginsReturnsArray()
    {
        using var engine = new Engine();
        var plugins = engine.ScanPlugins();
        Assert.NotNull(plugins);
        // May be empty but should not throw
    }

    [Fact]
    public void GetPresetsReturnsArrayWhenNoBackend()
    {
        using var engine = new Engine();
        var presets = engine.GetPresets();
        Assert.NotNull(presets);
        Assert.Empty(presets);
    }

    [Fact]
    public void GetErrorIsNullOnFreshThread()
    {
        // Error detail is thread-local; a brand-new thread has none.
        string? observed = "sentinel";
        var t = new Thread(() => observed = Engine.GetError());
        t.Start();
        t.Join();
        Assert.Null(observed);
    }

    [Fact]
    public void DoubleDisposeIsSafe()
    {
        var engine = new Engine();
        engine.Dispose();
        engine.Dispose(); // should not throw
    }
}
