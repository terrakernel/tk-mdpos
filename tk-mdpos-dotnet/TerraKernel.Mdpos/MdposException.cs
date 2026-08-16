namespace TerraKernel.Mdpos;

/// <summary>Why a render failed.</summary>
public enum MdposError
{
    /// <summary>
    /// The template was rejected. <see cref="Exception.Message"/> carries the source line
    /// and is meant for whoever edits the template.
    /// </summary>
    Template = -1,

    /// <summary>The template bytes were not valid UTF-8.</summary>
    InvalidUtf8 = -2,

    /// <summary>A required argument was null.</summary>
    NullArgument = -3,

    /// <summary>A profile field held a value this build does not define.</summary>
    InvalidProfile = -4,

    /// <summary>A panic was caught at the ABI boundary. This is a bug in mdpos.</summary>
    Panic = -99,
}

/// <summary>
/// Thrown when a template cannot be rendered.
/// </summary>
/// <remarks>
/// Deliberately one exception type rather than a hierarchy. Every template rejection comes
/// back as <see cref="MdposError.Template"/> with the offending source line already in the
/// message; typed exceptions per syntax error would be surface area for no gain, because
/// the useful half of the report is the text, not the code.
/// </remarks>
public sealed class MdposException : Exception
{
    /// <summary>The category of failure.</summary>
    public MdposError Error { get; }

    /// <summary>The raw status code returned across the C ABI.</summary>
    public int StatusCode { get; }

    internal MdposException(int statusCode, string message) : base(message)
    {
        StatusCode = statusCode;
        Error = (MdposError)statusCode;
    }
}
