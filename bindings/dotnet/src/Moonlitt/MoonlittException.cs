using System;

namespace Moonlitt;

/// <summary>
/// Exception thrown when a moonlitt native operation fails.
/// </summary>
public class MoonlittException : Exception
{
    public MoonlittException(string message) : base(message) { }
    public MoonlittException(string message, Exception inner) : base(message, inner) { }
}
