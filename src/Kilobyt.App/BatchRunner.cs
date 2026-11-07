using Kilobyt.Core;

namespace Kilobyt.App;

/// <summary>Per-file state as shown in the Status column (PRD §8).</summary>
public enum FileState
{
    Queued,
    Working,
    Done,
    Failed,
    Cancelled,
}

/// <summary>One row update, pushed to the UI through <see cref="IProgress{T}"/>.</summary>
public sealed record FileProgress(
    int Index,
    string FilePath,
    FileState State,
    string StatusText,
    string ResultText,
    string? ErrorCode = null,
    string? ErrorDetail = null,
    string? OutputPath = null,
    long SourceBytes = 0);

/// <summary>End-of-run summary for the status line and the completion modal.</summary>
public sealed record BatchSummary(
    int Total,
    int Done,
    int Failed,
    int Cancelled,
    string? LastOutputDirectory,
    IReadOnlyList<string> Failures)
{
    public int Completed => Done + Failed + Cancelled;
}

/// <summary>
/// Sequential batch driver (PRD §5.5). Contains no compression logic: every decision
/// about pixels/size/quality is delegated to <see cref="Compressor"/>; this class only
/// reads bytes, calls the core pipeline, verifies the file on disk and reports progress.
/// </summary>
public sealed class BatchRunner
{
    private readonly IJpegCodec _codec;

    public BatchRunner(IJpegCodec codec) => _codec = codec ?? throw new ArgumentNullException(nameof(codec));

    public async Task<BatchSummary> RunAsync(
        IReadOnlyList<string> paths,
        FitSettings settings,
        string? outputDirectory,
        IProgress<FileProgress>? progress,
        CancellationToken ct)
    {
        ArgumentNullException.ThrowIfNull(paths);
        ArgumentNullException.ThrowIfNull(settings);

        var reserved = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        var failures = new List<string>();
        int done = 0;
        int failed = 0;
        int cancelled = 0;
        string? lastOutputDirectory = null;
        long targetBytes = Math.Max(1, settings.TargetKb) * 1000L;

        for (int index = 0; index < paths.Count; index++)
        {
            string path = paths[index];

            if (ct.IsCancellationRequested)
            {
                cancelled++;
                Report(progress, index, path, FileState.Cancelled, "Cancelled", "—", 0);
                continue;
            }

            long sourceBytes = 0;
            Report(progress, index, path, FileState.Working, "Working…", "—", sourceBytes);

            // Never throw out of the loop: a bad file is a row, not a crash (§5.5).
            try
            {
                FileInfo info = new(path);
                if (!info.Exists)
                {
                    fail(ErrorCodes.Decode, "File is gone.");
                    continue;
                }

                sourceBytes = info.Length;
                byte[] input = await File.ReadAllBytesAsync(path, ct).ConfigureAwait(false);

                FitResult fit = Compressor.Fit(path, input, _codec, settings, ct);
                if (!fit.Success)
                {
                    fail(fit.ErrorCode ?? ErrorCodes.Decode, fit.ErrorDetail ?? ErrorMessages.Decode);
                    continue;
                }

                string destination = ResolveDestination(path, outputDirectory);
                string stem = Path.GetFileNameWithoutExtension(path);
                CommitResult commit = Compressor.Commit(fit.Jpeg!, destination, stem, reserved, ct);
                if (!commit.Ok)
                {
                    fail(commit.ErrorCode ?? ErrorCodes.Access, commit.ErrorDetail ?? ErrorMessages.Access(destination));
                    continue;
                }

                // §5.2.11: the size invariant is re-verified from disk, not from memory.
                long written = new FileInfo(commit.Path!).Length;
                if (written > targetBytes)
                {
                    TryDelete(commit.Path!);
                    fail(ErrorCodes.Unstable, ErrorMessages.Unstable);
                    continue;
                }

                done++;
                lastOutputDirectory = destination;
                progress?.Report(new FileProgress(
                    index,
                    path,
                    FileState.Done,
                    "Done",
                    DescribeResult(written, fit.Width, fit.Height),
                    OutputPath: commit.Path,
                    SourceBytes: sourceBytes));
            }
            catch (OperationCanceledException)
            {
                cancelled++;
                Report(progress, index, path, FileState.Cancelled, "Cancelled", "—", sourceBytes);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or NotSupportedException)
            {
                fail(ErrorCodes.Access, $"Cannot read this file — {ex.Message}");
            }
            catch (Exception)
            {
                fail(ErrorCodes.Decode, ErrorMessages.Decode);
            }

            void fail(string code, string reason)
            {
                failed++;
                failures.Add($"{Path.GetFileName(path)} \u2014 {reason}");
                progress?.Report(new FileProgress(index, path, FileState.Failed, $"Failed: {reason}", "\u2014", code, reason, SourceBytes: sourceBytes));
            }
        }

        return new BatchSummary(paths.Count, done, failed, cancelled, lastOutputDirectory, failures);
    }

    /// <summary>PRD §8 Result column: <c>42 KB · 600×450</c>, 1 KB = 1000 bytes.</summary>
    public static string DescribeResult(long bytes, int width, int height)
        => $"{FormatKb(bytes)} KB · {width}×{height}";

    public static string FormatKb(long bytes)
        => (bytes / 1000.0).ToString("0.#", System.Globalization.CultureInfo.InvariantCulture);

    private static string ResolveDestination(string sourcePath, string? outputDirectory)
    {
        if (!string.IsNullOrWhiteSpace(outputDirectory)) return outputDirectory!;
        return Path.GetDirectoryName(sourcePath) ?? Directory.GetCurrentDirectory();
    }

    private static void Report(IProgress<FileProgress>? progress, int index, string path, FileState state, string status, string result, long sourceBytes)
        => progress?.Report(new FileProgress(index, path, state, status, result, SourceBytes: sourceBytes));

    private static void TryDelete(string path)
    {
        try
        {
            if (File.Exists(path)) File.Delete(path);
        }
        catch (IOException)
        {
        }
        catch (UnauthorizedAccessException)
        {
        }
    }
}
