using System;
using System.Collections.Generic;
using System.IO;
using System.Threading;
using Kilobyt.Core;
using Xunit;

namespace Kilobyt.Core.Tests;

public class CommitTests
{
    private static string TempDir()
    {
        string dir = Path.Combine(Path.GetTempPath(), "kilobyt-commit-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(dir);
        return dir;
    }

    [Fact]
    public void Commit_WritesFinalFileAndRemovesPart()
    {
        string dir = TempDir();
        byte[] jpeg = { 1, 2, 3, 4, 5 };

        CommitResult r = Compressor.Commit(jpeg, dir, "photo", new HashSet<string>(), CancellationToken.None);

        Assert.True(r.Ok);
        Assert.Equal(Path.Combine(dir, "photo_fit.jpg"), r.Path);
        Assert.Equal(jpeg, File.ReadAllBytes(r.Path!));
        Assert.False(File.Exists(r.Path + ".part"));
        Assert.Single(Directory.GetFiles(dir));
    }

    [Fact]
    public void Commit_NeverOverwrites_SecondStemGetsDashTwo()
    {
        string dir = TempDir();
        var reserved = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        byte[] first = { 1, 1, 1 };
        byte[] second = { 2, 2, 2 };

        CommitResult a = Compressor.Commit(first, dir, "a", reserved, CancellationToken.None);
        CommitResult b = Compressor.Commit(second, dir, "a", reserved, CancellationToken.None);

        Assert.True(a.Ok);
        Assert.True(b.Ok);
        Assert.EndsWith("a_fit.jpg", a.Path);
        Assert.EndsWith("a_fit-2.jpg", b.Path);
        Assert.Equal(first, File.ReadAllBytes(a.Path!));
        Assert.Equal(second, File.ReadAllBytes(b.Path!));
    }

    [Fact]
    public void Commit_MissingDirectory_IsAccessError()
    {
        string dir = Path.Combine(Path.GetTempPath(), "kilobyt-missing-" + Guid.NewGuid().ToString("N"));

        CommitResult r = Compressor.Commit(new byte[] { 1 }, dir, "a", new HashSet<string>(), CancellationToken.None);

        Assert.False(r.Ok);
        Assert.Equal(ErrorCodes.Access, r.ErrorCode);
        Assert.Contains("folder is read-only or missing", r.ErrorDetail);
    }

    [Fact]
    public void Commit_DirectoryThatIsAFile_IsAccessError()
    {
        string dir = TempDir();
        string file = Path.Combine(dir, "notadir");
        File.WriteAllBytes(file, new byte[] { 1 });

        CommitResult r = Compressor.Commit(new byte[] { 1 }, file, "a", new HashSet<string>(), CancellationToken.None);

        Assert.False(r.Ok);
        Assert.Equal(ErrorCodes.Access, r.ErrorCode);
    }

    [Fact]
    public void Commit_Cancelled_WritesNothing()
    {
        string dir = TempDir();
        using var cts = new CancellationTokenSource();
        cts.Cancel();

        CommitResult r = Compressor.Commit(new byte[] { 1 }, dir, "a", new HashSet<string>(), cts.Token);

        Assert.False(r.Ok);
        Assert.Equal(ErrorCodes.Cancelled, r.ErrorCode);
        Assert.Empty(Directory.GetFiles(dir));
    }

    [Fact]
    public void Commit_NameExhausted_IsNameError()
    {
        string dir = TempDir();
        var reserved = new HashSet<string>(StringComparer.OrdinalIgnoreCase) { Path.Combine(dir, "a_fit.jpg") };
        for (int i = 2; i <= 999; i++)
            reserved.Add(Path.Combine(dir, $"a_fit-{i}.jpg"));

        CommitResult r = Compressor.Commit(new byte[] { 1 }, dir, "a", reserved, CancellationToken.None);

        Assert.False(r.Ok);
        Assert.Equal(ErrorCodes.Name, r.ErrorCode);
        Assert.Contains("rename the source files", r.ErrorDetail);
        Assert.Empty(Directory.GetFiles(dir));
    }

    [Fact]
    public void CommitVerified_WritesWhenTheProbeSizeHolds()
    {
        string dir = TempDir();
        var image = TestImages.Noise(100, 100, 40);
        var codec = new FakeJpegCodec(image) { SizeFactor = 0.05 };
        var fit = new FitResult(true, new byte[2500], 90, 100, 100, 3, 2500, null, null);

        FitResult r = Compressor.CommitVerified(
            fit, codec, image, 96, 3000, dir, "a", new HashSet<string>(), CancellationToken.None);

        Assert.True(r.Success);
        Assert.True(r.Jpeg!.LongLength <= 3000);
        Assert.Equal(90, r.Quality);
        Assert.Single(Directory.GetFiles(dir));
    }

    [Fact] // §5.2.11: oversized encode -> decrement q and retry
    public void CommitVerified_DecrementsQualityUntilItFits()
    {
        string dir = TempDir();
        var image = TestImages.Noise(100, 100, 41);
        var codec = new FakeJpegCodec(image) { SizeFactor = 0.03 };
        var fit = new FitResult(true, new byte[4000], 90, 100, 100, 3, 4000, null, null);

        FitResult r = Compressor.CommitVerified(
            fit, codec, image, 96, 3500, dir, "a", new HashSet<string>(), CancellationToken.None);

        Assert.True(r.Success);
        Assert.True(r.Quality < 90);
        Assert.True(r.Jpeg!.LongLength <= 3500);
        Assert.True(codec.EncodeCalls >= 1);
    }

    [Fact] // §5.2.11: still oversized after 3 retries -> E_UNSTABLE, no file
    public void CommitVerified_UnstableAfterThreeAttempts()
    {
        string dir = TempDir();
        var image = TestImages.Noise(100, 100, 42);
        var codec = new FakeJpegCodec(image);
        var fit = new FitResult(true, new byte[30_000], 90, 100, 100, 3, 30_000, null, null);

        FitResult r = Compressor.CommitVerified(
            fit, codec, image, 96, 2000, dir, "a", new HashSet<string>(), CancellationToken.None);

        Assert.False(r.Success);
        Assert.Null(r.Jpeg);
        Assert.Equal(ErrorCodes.Unstable, r.ErrorCode);
        Assert.Equal("Encoder produced an oversized file 3 times; skipped.", r.ErrorDetail);
        Assert.Empty(Directory.GetFiles(dir));
    }

    [Fact]
    public void CommitVerified_NeverEncodesBelowQ30()
    {
        string dir = TempDir();
        var image = TestImages.Noise(50, 50, 43);
        var codec = new FakeJpegCodec(image);
        var fit = new FitResult(true, new byte[9000], 30, 50, 50, 1, 9000, null, null);

        FitResult r = Compressor.CommitVerified(
            fit, codec, image, 96, 100, dir, "a", new HashSet<string>(), CancellationToken.None);

        Assert.False(r.Success);
        Assert.Equal(ErrorCodes.Unstable, r.ErrorCode);
        Assert.Equal(0, codec.EncodeCalls);
    }

    [Fact]
    public void CommitVerified_PassesThroughFailedFit()
    {
        var failed = new FitResult(false, null, 0, 10, 10, 0, 0, ErrorCodes.TooSmall, "nope");
        FitResult r = Compressor.CommitVerified(
            failed, new FakeJpegCodec(TestImages.Noise(4, 4, 44)), TestImages.Noise(4, 4, 44),
            96, 100, Path.GetTempPath(), "a", new HashSet<string>(), CancellationToken.None);

        Assert.Same(failed, r);
    }
}
