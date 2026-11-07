namespace Kilobyt.Core;

/// <summary>
/// PRD §5.1 input limits and §7 E_FORMAT. Extension checking lives in Core (not the shell)
/// so the rule is unit-testable on Linux without a file dialog.
/// </summary>
public static class FormatLimits
{
    public const long MaxFileBytes = 200_000_000L;
    public const long MaxPixels = 80_000_000L;

    private static readonly string[] Extensions = { ".jpg", ".jpeg", ".png", ".bmp" };

    public static IReadOnlyList<string> SupportedExtensions => Extensions;

    public static bool IsSupportedExtension(string? fileName)
    {
        if (string.IsNullOrWhiteSpace(fileName)) return false;
        string ext = Path.GetExtension(fileName);
        foreach (string candidate in Extensions)
        {
            if (string.Equals(ext, candidate, StringComparison.OrdinalIgnoreCase)) return true;
        }
        return false;
    }

    /// <summary>
    /// Returns null when the input is acceptable, otherwise the failing <see cref="FitResult"/>
    /// (E_FORMAT, E_EMPTY or E_TOOBIG). <paramref name="pixelCount"/> may be 0 when unknown.
    /// </summary>
    public static FitResult? Validate(string? fileName, long byteLength, long pixelCount = 0)
    {
        if (fileName is not null && !IsSupportedExtension(fileName))
            return Make(ErrorCodes.Format, ErrorMessages.Format);
        if (byteLength <= 0)
            return Make(ErrorCodes.Empty, ErrorMessages.Empty);
        if (byteLength > MaxFileBytes)
            return Make(ErrorCodes.TooBig, ErrorMessages.TooBigBytes(byteLength));
        if (pixelCount > MaxPixels)
            return Make(ErrorCodes.TooBig, ErrorMessages.TooBigMegaPixels(pixelCount));
        return null;
    }

    private static FitResult Make(string code, string detail)
        => new(false, null, 0, 0, 0, 0, 0, code, detail);
}
