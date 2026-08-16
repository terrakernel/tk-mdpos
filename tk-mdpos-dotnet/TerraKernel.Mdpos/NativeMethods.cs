using System.Runtime.InteropServices;

namespace TerraKernel.Mdpos;

/// <summary>
/// Raw bindings to the tk_mdpos C ABI. Kept internal: every buffer this layer hands back
/// carries a manual free obligation, and the whole point of the wrapper is that no caller
/// outside this assembly ever holds one.
/// </summary>
/// <remarks>
/// Mirrors <c>tk-mdpos-ffi/include/tk_mdpos.h</c> by hand, exactly as that header mirrors
/// <c>tk-mdpos-ffi/src/lib.rs</c> by hand. If you change one, change the others.
/// <para>
/// <c>DllImport("tk_mdpos")</c> resolves <c>tk_mdpos.dll</c> on Windows and
/// <c>libtk_mdpos.so</c> on Linux with no per-platform naming, which is the <c>tk-</c>
/// crate-naming convention reaching into the C ABI and paying off.
/// </para>
/// </remarks>
internal static unsafe partial class NativeMethods
{
    private const string Library = "tk_mdpos";

    // --- status codes ------------------------------------------------------------------

    internal const int TK_MDPOS_OK = 0;
    internal const int TK_MDPOS_ERR_TEMPLATE = -1;
    internal const int TK_MDPOS_ERR_INVALID_UTF8 = -2;
    internal const int TK_MDPOS_ERR_NULL_ARG = -3;
    internal const int TK_MDPOS_ERR_INVALID_PROFILE = -4;
    internal const int TK_MDPOS_ERR_PANIC = -99;

    /// <summary>
    /// An owned buffer produced by mdpos. Must go back through <see cref="tk_mdpos_free"/>
    /// and never through <c>free()</c> — it was allocated by Rust's allocator.
    /// </summary>
    /// <remarks>
    /// <c>Len</c> excludes the trailing NUL that always sits at <c>Ptr[Len]</c>. <c>Cap</c>
    /// is opaque and exists only so the Rust side can reconstruct the allocation exactly;
    /// it is why freeing takes the whole struct by value rather than just a pointer, and
    /// therefore why a stock <c>SafeHandle</c> (which tracks one IntPtr) does not fit.
    /// </remarks>
    [StructLayout(LayoutKind.Sequential)]
    internal struct TkMdposBuf
    {
        public byte* Ptr;
        public nuint Len;
        public nuint Cap;
    }

    [LibraryImport(Library)]
    internal static partial PrinterProfile tk_mdpos_profile_epson_80mm();

    [LibraryImport(Library)]
    internal static partial ushort tk_mdpos_columns(PrinterProfile* profile, byte mag);

    [LibraryImport(Library)]
    internal static partial uint tk_mdpos_format_version();

    [LibraryImport(Library)]
    internal static partial byte* tk_mdpos_version();

    [LibraryImport(Library)]
    internal static partial int tk_mdpos_render(
        byte* tmpl, nuint tmplLen, PrinterProfile* profile, TkMdposBuf* @out);

    [LibraryImport(Library)]
    internal static partial int tk_mdpos_preview(
        byte* tmpl, nuint tmplLen, PrinterProfile* profile, TkMdposBuf* @out);

    [LibraryImport(Library)]
    internal static partial int tk_mdpos_preview_html(
        byte* tmpl, nuint tmplLen, PrinterProfile* profile, TkMdposBuf* @out);

    [LibraryImport(Library)]
    internal static partial void tk_mdpos_free(TkMdposBuf buf);
}
