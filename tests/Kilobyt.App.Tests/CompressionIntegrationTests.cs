using System.Drawing;
using System.Security.Cryptography;
using Kilobyt.Core;
using Xunit;

namespace Kilobyt.App.Tests;

/// <summary>
/// Windows-only integration tests (PRD §11). They exercise the real System.Drawing codec
/// and real files on disk; they run on windows-latest only.
/// </summary>
public sealed class CompressionIntegrationTests
{
    private static readonly IJpegCodec Codec = new SystemDrawingJpegCodec();

    // ------------------------------------------------------------ AC-1.1

    [Fact]
    public void AC1_1_4000x3000_jpeg_fits_100KB_and_decodes()
    {
        using var temp = new TempDir("kilobyt-ac11");
        string source = temp.File("beach.jpg");

        using (Bitmap bitmap = TestImages.PhotoLike(4000, 3000, seed: 7))
        {
            File.WriteAllBytes(source, TestImages.JpegBytes(bitmap, 90));
        }

        byte[] input = File.ReadAllBytes(source);
        FitResult fit = Compressor.Fit(source, input, Codec, new FitSettings(100, null, null), CancellationToken.None);
        Assert.True(fit.Success, fit.ErrorDetail);
        Assert.InRange(fit.Quality, 30, 95);
        Assert.InRange(fit.Probes, 1, Compressor.ProbeBudget);

        var reserved = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        CommitResult commit = Compressor.Commit(fit.Jpeg!, temp.Path, "beach", reserved, CancellationToken.None);
        Assert.True(commit.Ok, commit.ErrorDetail);
        Assert.Equal(temp.File("beach_fit.jpg"), commit.Path);

        var written = new FileInfo(commit.Path!);
        Assert.True(written.Exists, "output file must exist");
        Assert.True(written.Length <= 100_000, $"output was {written.Length} bytes");

        using Image decoded = TestImages.DecodeOrThrow(File.ReadAllBytes(written.FullName));
        Assert.True(decoded.Width <= 4000 && decoded.Height <= 3000);
        Assert.InRange(decoded.Width, 1, 4000);

        double sourceAspect = 4000.0 / 3000.0;
        double outputAspect = (double)decoded.Width / decoded.Height;
        double widthDrift = Math.Abs((decoded.Width / sourceAspect) - decoded.Height);
        Assert.True(widthDrift <= 1.0, $"aspect drift {widthDrift:0.###} px");
        Assert.True(Math.Abs(outputAspect - sourceAspect) < 0.02);
    }

    // ------------------------------------------------------------ AC-4.2

    [Fact]
    public void AC4_2_output_has_no_exif_app1_or_gps()
    {
        using var temp = new TempDir("kilobyt-ac42");
        string source = temp.File("gps.jpg");

        using (Bitmap bitmap = TestImages.Solid(1200, 900, Color.SteelBlue))
        {
            byte[] plain = TestImages.JpegBytes(bitmap, 92);
            byte[] withExif = ExifInjector.WithExif(plain, orientation: 1, gps: true);
            Assert.True(JpegInspector.HasApp1(withExif), "fixture must actually carry an APP1 segment");
            Assert.True(JpegInspector.ContainsAscii(withExif, ExifInjector.MakeSentinel));
            File.WriteAllBytes(source, withExif);
        }

        FitResult fit = Compressor.Fit(source, File.ReadAllBytes(source), Codec,
            new FitSettings(120, null, null), CancellationToken.None);
        Assert.True(fit.Success, fit.ErrorDetail);

        byte[] output = fit.Jpeg!;
        Assert.False(JpegInspector.HasApp1(output), "output must not contain an APP1 segment");
        Assert.False(JpegInspector.ContainsAscii(output, "Exif"), "output must not contain the EXIF signature");
        Assert.False(JpegInspector.ContainsAscii(output, "Exif\0\0"), "output must not contain the EXIF header");
        Assert.False(JpegInspector.ContainsAscii(output, ExifInjector.MakeSentinel), "camera make leaked");
        Assert.False(JpegInspector.ContainsAscii(output, ExifInjector.SerialSentinel), "serial leaked");
        Assert.False(JpegInspector.ContainsAscii(output, "2026:11:07 12:34:56"), "DateTimeOriginal leaked");

        IReadOnlyList<byte> markers = JpegInspector.Markers(output);
        Assert.DoesNotContain((byte)0xE1, markers); // EXIF / XMP
        Assert.DoesNotContain((byte)0xE2, markers); // ICC profile
        Assert.DoesNotContain((byte)0xED, markers); // IPTC / Photoshop
        Assert.DoesNotContain((byte)0xFE, markers); // COM

        // GPS payload: 51/1 30/1 1234/100 and 0/1 7/1 9876/100 as little-endian rationals.
        byte[] gpsLatitude = BitConverter.GetBytes(51u).Concat(BitConverter.GetBytes(1u)).ToArray();
        Assert.False(Contains(output, gpsLatitude), "GPS latitude bytes leaked");

        using Image decoded = TestImages.DecodeOrThrow(output);
        Assert.Equal(1200, decoded.Width);
        Assert.Equal(900, decoded.Height);
    }

    // ------------------------------------------------------------ AC-4.3

    [Fact]
    public void AC4_3_orientation6_is_baked_into_the_pixels()
    {
        using var temp = new TempDir("kilobyt-ac43");
        string source = temp.File("portrait.jpg");

        using (Bitmap bitmap = TestImages.Solid(400, 300, Color.CadetBlue))
        {
            byte[] plain = TestImages.JpegBytes(bitmap, 92);
            File.WriteAllBytes(source, ExifInjector.WithExif(plain, orientation: 6, gps: false));
        }

        byte[] input = File.ReadAllBytes(source);
        Rgb24Image raw = Codec.Decode(input, out ImageMeta meta);
        Assert.Equal(6, meta.Orientation);
        Assert.Equal(400, raw.Width);
        Assert.Equal(300, raw.Height);

        FitResult fit = Compressor.Fit(source, input, Codec, new FitSettings(100, null, null), CancellationToken.None);
        Assert.True(fit.Success, fit.ErrorDetail);

        // 400x300 stored, tagged "rotate 90° CW" => displayed 300x400.
        Assert.True(fit.Width < fit.Height, $"expected a portrait output, got {fit.Width}x{fit.Height}");
        Assert.Equal(300, fit.Width);
        Assert.Equal(400, fit.Height);

        using Image decoded = TestImages.DecodeOrThrow(fit.Jpeg!);
        Assert.True(decoded.Width < decoded.Height);
        Assert.Equal(300, decoded.Width);
        Assert.Equal(400, decoded.Height);

        // The rotation is baked in, so the output carries no orientation tag at all.
        Assert.False(JpegInspector.HasApp1(fit.Jpeg!));
    }

    // ------------------------------------------------------------ AC-7.1

    [Fact]
    public void AC7_1_transparent_png_flattens_to_white()
    {
        using var temp = new TempDir("kilobyt-ac71");
        string source = temp.File("logo.png");

        using (Bitmap bitmap = TestImages.Transparent(64, 64))
        {
            File.WriteAllBytes(source, TestImages.PngBytes(bitmap));
        }

        Rgb24Image image = Codec.Decode(File.ReadAllBytes(source), out _);
        Assert.Equal(64, image.Width);
        Assert.Equal(64, image.Height);
        Assert.Equal(64 * 64 * 3, image.Pixels.Length);

        for (int i = 0; i < image.Pixels.Length; i++)
        {
            Assert.True(image.Pixels[i] >= 250, $"pixel byte {i} was {image.Pixels[i]}, expected near-white");
        }

        // And it survives the full pipeline.
        FitResult fit = Compressor.Fit(source, File.ReadAllBytes(source), Codec,
            new FitSettings(50, null, null), CancellationToken.None);
        Assert.True(fit.Success, fit.ErrorDetail);
        using Image decoded = TestImages.DecodeOrThrow(fit.Jpeg!);
        Assert.Equal(64, decoded.Width);
    }

    // ------------------------------------------------------------ AC-8.1

    [Fact]
    public void AC8_1_same_input_twice_is_byte_identical()
    {
        using var temp = new TempDir("kilobyt-ac81");
        string source = temp.File("repeat.jpg");

        using (Bitmap bitmap = TestImages.PhotoLike(1600, 1200, seed: 21))
        {
            File.WriteAllBytes(source, TestImages.JpegBytes(bitmap, 92));
        }

        string first = RunToDisk(temp.File("run1"), source, 80);
        string second = RunToDisk(temp.File("run2"), source, 80);

        string hash1 = Sha256(first);
        string hash2 = Sha256(second);
        Assert.Equal(hash1, hash2);
        Assert.True(new FileInfo(first).Length <= 80_000);
    }

    // ------------------------------------------------------------ AC-3.1

    [Fact]
    public async Task AC3_1_batch_of_40_with_3_corrupt_writes_37_and_reports_3()
    {
        using var temp = new TempDir("kilobyt-ac31");

        var sources = new List<string>();
        for (int i = 0; i < 40; i++)
        {
            string path = temp.File($"photo-{i:00}.jpg");
            using (Bitmap bitmap = TestImages.Noise(320, 240, seed: 100 + i))
            {
                File.WriteAllBytes(path, TestImages.JpegBytes(bitmap, 88));
            }

            sources.Add(path);
        }

        // Three files that are not images at all (truncated garbage with a .jpg name).
        for (int i = 0; i < 3; i++)
        {
            string path = temp.File($"broken-{i}.jpg");
            File.WriteAllBytes(path, Guid.NewGuid().ToByteArray().Concat(new byte[] { 0xFF, 0xD8, 0x00, 0x11 }).ToArray());
            sources.Add(path);
        }

        var runner = new BatchRunner(Codec);
        var updates = new List<FileProgress>();
        var progress = new InlineProgress(updates);

        BatchSummary summary = await runner.RunAsync(
            sources,
            new FitSettings(100, null, null),
            outputDirectory: null,
            progress,
            CancellationToken.None);

        Assert.Equal(43, summary.Total);
        Assert.Equal(40, summary.Done);
        Assert.Equal(3, summary.Failed);
        Assert.Equal(0, summary.Cancelled);
        Assert.Equal(3, summary.Failures.Count);

        string[] outputs = Directory.GetFiles(temp.Path, "*_fit.jpg");
        Assert.Equal(40, outputs.Length);

        // No partial writes survived.
        Assert.Empty(Directory.GetFiles(temp.Path, "*.part"));

        // Progress must have been reported for every file, with a specific failure reason.
        Assert.Equal(43, updates.Count(u => u.State == FileState.Working));
        Assert.Equal(43, updates.Count(u => u.State is FileState.Done or FileState.Failed or FileState.Cancelled));
        var failed = updates.Where(u => u.State == FileState.Failed).ToList();
        Assert.Equal(3, failed.Count);
        Assert.All(failed, f => Assert.StartsWith("Failed: ", f.StatusText));
        Assert.All(failed, f => Assert.False(string.IsNullOrWhiteSpace(f.ErrorDetail)));

        foreach (string output in outputs)
        {
            Assert.True(new FileInfo(output).Length <= 100_000, $"{output} exceeded the target");
        }
    }

    // ------------------------------------------------------------- helpers

    private static string RunToDisk(string directory, string source, int targetKb)
    {
        Directory.CreateDirectory(directory);
        byte[] input = File.ReadAllBytes(source);
        FitResult fit = Compressor.Fit(source, input, Codec, new FitSettings(targetKb, null, null), CancellationToken.None);
        Assert.True(fit.Success, fit.ErrorDetail);

        CommitResult commit = Compressor.Commit(
            fit.Jpeg!,
            directory,
            Path.GetFileNameWithoutExtension(source),
            new HashSet<string>(StringComparer.OrdinalIgnoreCase),
            CancellationToken.None);
        Assert.True(commit.Ok, commit.ErrorDetail);
        return commit.Path!;
    }

    private static string Sha256(string path)
    {
        using FileStream stream = File.OpenRead(path);
        return Convert.ToHexString(SHA256.HashData(stream));
    }

    private static bool Contains(byte[] haystack, byte[] needle)
        => needle.Length > 0 && haystack.AsSpan().IndexOf(needle) >= 0;

    /// <summary>Synchronous collector: xunit's context would defer Progress&lt;T&gt; callbacks.</summary>
    private sealed class InlineProgress : IProgress<FileProgress>
    {
        private readonly List<FileProgress> _sink;

        public InlineProgress(List<FileProgress> sink) => _sink = sink;

        public void Report(FileProgress value)
        {
            lock (_sink) _sink.Add(value);
        }
    }
}
