namespace Moonlitt;

/// <summary>
/// Describes a MIDI input device discovered by <see cref="Runtime.ListMidiInputs"/>.
/// </summary>
public sealed record MidiDeviceInfo(int Id, string Name);
