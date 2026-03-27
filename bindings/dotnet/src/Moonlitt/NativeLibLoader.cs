using System;
using System.IO;
using System.Runtime.InteropServices;

namespace Moonlitt;

/// <summary>
/// Locates and loads the moonlitt_ffi native library from well-known paths.
/// Call <see cref="EnsureLoaded"/> once before any P/Invoke call.
/// Thread-safe via lock.
/// </summary>
public static class NativeLibLoader
{
    private static IntPtr _handle;
    private static volatile bool _registered;
    private static readonly object _lock = new();

    public static void EnsureLoaded()
    {
        if (_registered) return;

        lock (_lock)
        {
            if (_registered) return;

            var searchPaths = GetSearchPaths();
            foreach (var path in searchPaths)
            {
                if (File.Exists(path) && NativeLibrary.TryLoad(path, out _handle))
                {
                    NativeLibrary.SetDllImportResolver(
                        typeof(NativeLibLoader).Assembly,
                        (name, assembly, searchPath) =>
                            name == "moonlitt_ffi" ? _handle : IntPtr.Zero);
                    _registered = true;
                    return;
                }
            }
        }
    }

    private static string[] GetSearchPaths()
    {
        var assemblyDir = Path.GetDirectoryName(typeof(NativeLibLoader).Assembly.Location) ?? ".";
        var subDir = RuntimeInformation.IsOSPlatform(OSPlatform.Windows) ? "win-x64"
            : RuntimeInformation.IsOSPlatform(OSPlatform.OSX) ? "osx"
            : "linux-x64";
        var ext = RuntimeInformation.IsOSPlatform(OSPlatform.Windows) ? ".dll"
            : RuntimeInformation.IsOSPlatform(OSPlatform.OSX) ? ".dylib"
            : ".so";
        var prefix = RuntimeInformation.IsOSPlatform(OSPlatform.Windows) ? "" : "lib";
        var libName = $"{prefix}moonlitt_ffi{ext}";

        return new[]
        {
            Path.Combine(assemblyDir, "native", subDir, libName),
            Path.Combine(assemblyDir, libName),
            libName, // system path fallback
        };
    }
}
