namespace Kilobyt.Core;

/// <summary>Interleaved RGB, 3 bytes per pixel, row-major, no padding.</summary>
public readonly record struct Rgb24Image(int Width, int Height, byte[] Pixels);

/// <summary>Metadata reported by the codec at decode time (PRD §5.2.2).</summary>
public sealed record ImageMeta(int DpiX, int DpiY, int Orientation);

/// <summary>
/// Platform codec boundary. Implemented in Kilobyt.App with System.Drawing; Core never
/// references a native image library (PRD §5.8, risk R1).
/// </summary>
public interface IJpegCodec
{
    /// <summary>Decode to RGB24, compositing alpha over white. Throws <see cref="UnsupportedImageException"/>.</summary>
    Rgb24Image Decode(byte[] file, out ImageMeta meta);

    /// <summary>Encode at a quality in [30,95] with a JFIF density of <paramref name="dpi"/>.</summary>
    byte[] Encode(Rgb24Image image, int quality, int dpi);
}

/// <summary>Raised by an <see cref="IJpegCodec"/> for corrupt or unsupported input.</summary>
public sealed class UnsupportedImageException : Exception
{
    public UnsupportedImageException() { }
    public UnsupportedImageException(string message) : base(message) { }
    public UnsupportedImageException(string message, Exception inner) : base(message, inner) { }
}

/// <summary>Raised by <see cref="OutputNamer"/> after 999 collisions (E_NAME).</summary>
public sealed class NameCollisionException : IOException
{
    public NameCollisionException(string message) : base(message) { }
}

/// <summary>User settings for one file. TargetKb uses 1 KB = 1000 bytes (PRD §5.1).</summary>
public sealed record FitSettings(int TargetKb, int? MaxWidth, int? MaxHeight);

/// <summary>Outcome of one file. On failure <see cref="Jpeg"/> is null and no file may be written.</summary>
public sealed record FitResult(
    bool Success,
    byte[]? Jpeg,
    int Quality,
    int Width,
    int Height,
    int Probes,
    long BestBytes,
    string? ErrorCode,
    string? ErrorDetail);

/// <summary>Result of the on-disk write/verify/commit step (PRD §5.2.11-12).</summary>
public sealed record CommitResult(bool Ok, string? Path, string? ErrorCode, string? ErrorDetail);

/// <summary>Error codes exactly as named in PRD §7.</summary>
public static class ErrorCodes
{
    public const string Empty = "E_EMPTY";
    public const string Decode = "E_DECODE";
    public const string Format = "E_FORMAT";
    public const string TooBig = "E_TOOBIG";
    public const string TooSmall = "E_TOO_SMALL";
    public const string Access = "E_ACCESS";
    public const string Name = "E_NAME";
    public const string Unstable = "E_UNSTABLE";
    public const string Budget = "E_BUDGET";
    public const string Cancelled = "E_CANCELLED";
}

/// <summary>Message text exactly as specified in PRD §7. Culture-invariant by construction.</summary>
public static class ErrorMessages
{
    public const string Empty = "Empty file.";
    public const string Decode = "Cannot read this image — the file may be damaged or use an unsupported format.";
    public const string Format = "Only JPG, PNG and BMP are supported. Convert HEIC/TIFF/WebP first.";
    public const string Unstable = "Encoder produced an oversized file 3 times; skipped.";
    public const string Budget = "Internal error (E_BUDGET) — please report this file.";
    public const string Cancelled = "Cancelled.";

    public static string TooBigMegaPixels(long pixels)
        => $"Too large: {(pixels / 1_000_000.0).ToString("0.#", System.Globalization.CultureInfo.InvariantCulture)} MP. Limit is 80 MP.";

    public static string TooBigBytes(long bytes)
        => $"Too large: {(bytes / 1_000_000.0).ToString("0.#", System.Globalization.CultureInfo.InvariantCulture)} MB. Limit is 200 MB.";

    public static string TooSmall(long targetBytes, long bestBytes, int width, int height)
        => $"Cannot reach {FormatKb(targetBytes)} KB. The smallest this photo can be is {FormatKb(bestBytes)} KB at {width}×{height}.";

    public static string Access(string directory)
        => $"Cannot write to {directory} — folder is read-only or missing.";

    public static string Name(string stem)
        => $"Too many files named {stem}_fit — rename the source files.";

    private static string FormatKb(long bytes)
        => (bytes / 1000.0).ToString("0.#", System.Globalization.CultureInfo.InvariantCulture);
}
