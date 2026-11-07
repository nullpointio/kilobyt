namespace Kilobyt.Core;

/// <summary>
/// PRD §5.2 pipeline. Encodes in memory only; the on-disk verify/commit step is
/// <see cref="Commit"/> / <see cref="CommitVerified"/> (added here so §5.2.11-12 is
/// unit-testable without the WinForms shell).
/// </summary>
public static class Compressor
{
    /// <summary>Hard cap on encode probes per file (PRD §5.3).</summary>
    public const int ProbeBudget = 200;

    public const int MaxDownscales = 8;
    public const int MinDimension = 64;
    private const double DownscaleFactor = 0.85;
    private const int DefaultDpi = 96;

    /// <summary>PRD §5.2 steps 1-13 as far as memory allows (no file is written).</summary>
    public static FitResult Fit(byte[] input, IJpegCodec codec, FitSettings settings, CancellationToken ct)
        => Fit(null, input, codec, settings, ct);

    /// <summary>
    /// Same pipeline, but <paramref name="fileName"/> is validated for E_FORMAT first.
    /// The shell passes the source path here; pass null to skip the extension rule.
    /// </summary>
    public static FitResult Fit(string? fileName, byte[] input, IJpegCodec codec, FitSettings settings, CancellationToken ct)
    {
        ArgumentNullException.ThrowIfNull(codec);
        ArgumentNullException.ThrowIfNull(settings);

        // 1. Read / validate input size.
        if (input is null || input.Length == 0)
            return Fail(ErrorCodes.Empty, ErrorMessages.Empty);
        if (fileName is not null && !FormatLimits.IsSupportedExtension(fileName))
            return Fail(ErrorCodes.Format, ErrorMessages.Format);
        if (input.LongLength > FormatLimits.MaxFileBytes)
            return Fail(ErrorCodes.TooBig, ErrorMessages.TooBigBytes(input.LongLength));
        if (ct.IsCancellationRequested)
            return Cancelled(0, 0, 0, 0);

        // 2. Decode.
        Rgb24Image decoded;
        ImageMeta meta;
        try
        {
            decoded = codec.Decode(input, out meta);
        }
        catch (OperationCanceledException)
        {
            return Cancelled(0, 0, 0, 0);
        }
        catch (Exception)
        {
            return Fail(ErrorCodes.Decode, ErrorMessages.Decode);
        }

        if (decoded.Width < 1 || decoded.Height < 1 || decoded.Pixels is null
            || decoded.Pixels.LongLength != (long)decoded.Width * decoded.Height * 3)
        {
            return Fail(ErrorCodes.Decode, ErrorMessages.Decode);
        }

        long pixels = (long)decoded.Width * decoded.Height;
        if (pixels > FormatLimits.MaxPixels)
            return Fail(ErrorCodes.TooBig, ErrorMessages.TooBigMegaPixels(pixels));

        // 3. Orient.
        Rgb24Image oriented = ExifOrientation.Apply(decoded, meta.Orientation);

        // 4. Plan dimensions (never upscale).
        (int planW, int planH) = SizePlanner.Fit(oriented.Width, oriented.Height, settings.MaxWidth, settings.MaxHeight);

        // 5. Resample once to the planned size; the downscale loop re-samples from here.
        Rgb24Image planned = planW == oriented.Width && planH == oriented.Height
            ? oriented
            : Resampler.Downscale(oriented, planW, planH);

        long targetBytes = Math.Max(1, settings.TargetKb) * 1000L;
        int dpi = ChooseDpi(meta);

        int probes = 0;
        long bestBytes = long.MaxValue;
        int bestQuality = 0;
        int bestW = planned.Width;
        int bestH = planned.Height;

        Rgb24Image buffer = planned;
        int downscales = 0;

        try
        {
            while (true)
            {
                if (ct.IsCancellationRequested) return Cancelled(buffer.Width, buffer.Height, probes, bestBytes);

                Rgb24Image current = buffer;
                int? quality = QualitySearch.Find(
                    q =>
                    {
                        if (probes >= ProbeBudget) throw new ProbeBudgetExceededException();
                        probes++;
                        long size = codec.Encode(current, q, dpi).LongLength;
                        if (size < bestBytes)
                        {
                            bestBytes = size;
                            bestQuality = q;
                            bestW = current.Width;
                            bestH = current.Height;
                        }
                        return size;
                    },
                    targetBytes,
                    ct);

                if (quality.HasValue)
                {
                    byte[] final = codec.Encode(buffer, quality.Value, dpi);
                    if (final.LongLength < bestBytes)
                    {
                        bestBytes = final.LongLength;
                        bestQuality = quality.Value;
                        bestW = buffer.Width;
                        bestH = buffer.Height;
                    }

                    return new FitResult(true, final, quality.Value, buffer.Width, buffer.Height,
                                         probes, final.LongLength, null, null);
                }

                // 7. Downscale loop: at most 8 steps, floor 64 px, height from the
                // planned aspect (which is the source aspect within 1 px).
                if (downscales >= MaxDownscales) break;

                int nextW = Math.Max(MinDimension, (int)Math.Floor(buffer.Width * DownscaleFactor));
                int nextH = (int)Math.Round(nextW * (double)planH / planW, MidpointRounding.AwayFromZero);

                if (buffer.Width <= MinDimension || buffer.Height <= MinDimension) break;
                if (nextW >= buffer.Width || nextH < MinDimension || nextH >= buffer.Height) break;

                if (ct.IsCancellationRequested) return Cancelled(buffer.Width, buffer.Height, probes, bestBytes);

                downscales++;
                buffer = Resampler.Downscale(oriented, nextW, nextH);
            }
        }
        catch (ProbeBudgetExceededException)
        {
            return new FitResult(false, null, 0, buffer.Width, buffer.Height, probes, SafeBest(bestBytes),
                                 ErrorCodes.Budget, ErrorMessages.Budget);
        }
        catch (OperationCanceledException)
        {
            return Cancelled(buffer.Width, buffer.Height, probes, bestBytes);
        }

        // 8. Unreachable target: state it, write nothing.
        long best = SafeBest(bestBytes);
        return new FitResult(false, null, bestQuality, bestW, bestH, probes, best,
                             ErrorCodes.TooSmall, ErrorMessages.TooSmall(targetBytes, best, bestW, bestH));
    }

    /// <summary>
    /// PRD §5.2.11-12 without a re-encode: write <c>.part</c>, flush, re-read the length,
    /// retry the write up to 3 times, then commit with <see cref="File.Move(string, string)"/>
    /// (never overwriting). Returns E_ACCESS / E_NAME / E_UNSTABLE / E_CANCELLED.
    /// </summary>
    public static CommitResult Commit(byte[] jpeg, string dir, string stem, ISet<string> reserved, CancellationToken ct)
    {
        if (jpeg is null) throw new ArgumentNullException(nameof(jpeg));
        if (dir is null) throw new ArgumentNullException(nameof(dir));
        if (stem is null) throw new ArgumentNullException(nameof(stem));

        if (ct.IsCancellationRequested)
            return new CommitResult(false, null, ErrorCodes.Cancelled, ErrorMessages.Cancelled);
        if (!Directory.Exists(dir))
            return new CommitResult(false, null, ErrorCodes.Access, ErrorMessages.Access(dir));

        string finalPath;
        try
        {
            finalPath = OutputNamer.NextFree(dir, stem, ".jpg", reserved);
        }
        catch (NameCollisionException ex)
        {
            return new CommitResult(false, null, ErrorCodes.Name, ex.Message);
        }

        string partPath = finalPath + ".part";
        try
        {
            bool verified = false;
            for (int attempt = 1; attempt <= 3 && !verified; attempt++)
            {
                if (ct.IsCancellationRequested) break;

                using (var stream = new FileStream(partPath, FileMode.Create, FileAccess.Write, FileShare.None))
                {
                    stream.Write(jpeg, 0, jpeg.Length);
                    stream.Flush(true);
                }

                var info = new FileInfo(partPath);
                verified = info.Exists && info.Length == jpeg.Length;
            }

            if (ct.IsCancellationRequested)
            {
                TryDelete(partPath);
                return new CommitResult(false, null, ErrorCodes.Cancelled, ErrorMessages.Cancelled);
            }

            if (!verified)
            {
                TryDelete(partPath);
                return new CommitResult(false, null, ErrorCodes.Unstable, ErrorMessages.Unstable);
            }

            File.Move(partPath, finalPath);
            return new CommitResult(true, finalPath, null, null);
        }
        catch (OperationCanceledException)
        {
            TryDelete(partPath);
            return new CommitResult(false, null, ErrorCodes.Cancelled, ErrorMessages.Cancelled);
        }
        catch (Exception ex) when (ex is IOException || ex is UnauthorizedAccessException || ex is NotSupportedException)
        {
            TryDelete(partPath);
            return new CommitResult(false, null, ErrorCodes.Access, ErrorMessages.Access(dir));
        }
    }

    /// <summary>
    /// PRD §5.2.11 with the q-1 retry loop: if the encoded length is still over
    /// <paramref name="targetBytes"/> re-encode at q-1 (max 3 attempts) then E_UNSTABLE.
    /// Added because §5.2.11 needs both the target and the encoder; <see cref="Commit"/>
    /// alone can only verify the bytes it was given.
    /// </summary>
    public static FitResult CommitVerified(
        FitResult fit,
        IJpegCodec codec,
        Rgb24Image image,
        int dpi,
        long targetBytes,
        string dir,
        string stem,
        ISet<string> reserved,
        CancellationToken ct)
    {
        ArgumentNullException.ThrowIfNull(fit);
        ArgumentNullException.ThrowIfNull(codec);

        if (!fit.Success || fit.Jpeg is null) return fit;

        byte[] bytes = fit.Jpeg;
        int quality = fit.Quality;

        for (int attempt = 1; bytes.LongLength > targetBytes; attempt++)
        {
            if (attempt > 3)
                return new FitResult(false, null, quality, fit.Width, fit.Height, fit.Probes,
                                     fit.BestBytes, ErrorCodes.Unstable, ErrorMessages.Unstable);
            if (ct.IsCancellationRequested)
                return new FitResult(false, null, quality, fit.Width, fit.Height, fit.Probes,
                                     fit.BestBytes, ErrorCodes.Cancelled, ErrorMessages.Cancelled);

            quality--;
            if (quality < QualitySearch.MinQuality)
                return new FitResult(false, null, fit.Quality, fit.Width, fit.Height, fit.Probes,
                                     fit.BestBytes, ErrorCodes.Unstable, ErrorMessages.Unstable);

            bytes = codec.Encode(image, quality, dpi);
        }

        CommitResult commit = Commit(bytes, dir, stem, reserved, ct);
        if (!commit.Ok)
            return new FitResult(false, null, quality, fit.Width, fit.Height, fit.Probes,
                                 bytes.LongLength, commit.ErrorCode, commit.ErrorDetail);

        return new FitResult(true, bytes, quality, fit.Width, fit.Height, fit.Probes,
                             bytes.LongLength, null, null);
    }

    private static int ChooseDpi(ImageMeta meta)
    {
        // §5.2.9: preserve each axis when in [1,1200]; the codec takes one density value,
        // so prefer DpiX, then DpiY, else 96.
        if (meta.DpiX >= 1 && meta.DpiX <= 1200) return meta.DpiX;
        if (meta.DpiY >= 1 && meta.DpiY <= 1200) return meta.DpiY;
        return DefaultDpi;
    }

    private static long SafeBest(long bestBytes) => bestBytes == long.MaxValue ? 0 : bestBytes;

    private static void TryDelete(string path)
    {
        try
        {
            if (File.Exists(path)) File.Delete(path);
        }
        catch (IOException)
        {
            // Best effort: a leftover .part must never mask the real error.
        }
        catch (UnauthorizedAccessException)
        {
        }
    }

    private static FitResult Fail(string code, string detail)
        => new(false, null, 0, 0, 0, 0, 0, code, detail);

    private static FitResult Cancelled(int width, int height, int probes, long bestBytes)
        => new(false, null, 0, width, height, probes, SafeBest(bestBytes), ErrorCodes.Cancelled, ErrorMessages.Cancelled);

    private sealed class ProbeBudgetExceededException : Exception
    {
    }
}
