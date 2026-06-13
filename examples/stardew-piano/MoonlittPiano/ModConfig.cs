namespace MoonlittPiano;

/// <summary>config.json — created next to the mod on first launch.</summary>
public sealed class ModConfig
{
    /// <summary>
    /// A .mlsession designed in the moonlitt desktop app. When set and
    /// valid, the game plays through it — instruments, Keyscape patch
    /// states, mixer and sends exactly as saved. Takes priority over
    /// <see cref="Sf2Path"/>.
    /// </summary>
    public string SessionPath { get; set; } = "";

    /// <summary>
    /// GM SoundFont fallback. Empty = auto-scan
    /// ~/Library/Audio/Sounds/Banks (GeneralUser preferred).
    /// </summary>
    public string Sf2Path { get; set; } = "";

    /// <summary>Master volume, 0.0–1.0.</summary>
    public float Volume { get; set; } = 0.9f;

    /// <summary>
    /// MIDI note for a flute block tuned all the way down (its pitch
    /// cycles 0–2300 in semitone steps, so 48 = C3..B4 range).
    /// </summary>
    public int FluteBaseNote { get; set; } = 48;

    /// <summary>MIDI channel for flute blocks (0-based; 9 = GM drums).</summary>
    public int FluteChannel { get; set; } = 0;

    /// <summary>Seconds a block note rings before its note-off.</summary>
    public float NoteSeconds { get; set; } = 2.0f;
}
