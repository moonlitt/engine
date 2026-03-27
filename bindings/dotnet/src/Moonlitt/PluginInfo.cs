namespace Moonlitt;

/// <summary>
/// Describes an audio plugin discovered by <see cref="Engine.ScanPlugins"/>.
/// </summary>
public sealed record PluginInfo(string Name, string Path, string Format);
