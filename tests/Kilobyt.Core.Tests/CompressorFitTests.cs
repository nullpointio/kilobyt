using System;
using System.Collections.Generic;
using System.Linq;
using System.Security.Cryptography;
using System.Threading;
using Kilobyt.Core;
using Xunit;

namespace Kilobyt.Core.Tests;

public class CompressorFitTests
{
    private static FitSettings Settings(int kb, int? maxW = null, int? maxH = null)
        => new(kb, maxW, maxH);

    [Fact] // AC-1.3: already smaller than target -> q95, dimensions unchanged, no upscale
    public void AlreadySmallerThanTarget_UsesQ95AndKeepsDimensions()
    {
        var image = TestImages.Noise(300, 200, 11);
        var codec = new FakeJpegCodec(image);

        FitResult r = Compressor.Fit(new byte[] { 1, 2, 3 }, codec, Settings(500), CancellationToken.None);

        Assert.True(r.Success);
        Assert.Equal(95, r.Quality);
        Assert.Equal(300, r.Width);
        Assert.Equal(200, r.Height);
        Assert.True(r.Jpeg!.Length <= 500_000);
        Assert.True(r.BestBytes <= 500_000);
        Assert.Null(r.ErrorCode);
    }

    [Fact] // AC-1.1 / AC-1.2: 12 MP noise, 30 KB target -> mid-ladder quality
    public void TwelveMegapixels_30Kb_FitsAndReportsSaneSearch()
    {
        var image = TestImages.Noise(4000, 3000, 12);
        var codec = new FakeJpegCodec(image) { SizeFactor = 0.002 };

        FitResult r = Compressor.Fit(new byte[] { 9 }, codec, Settings(30), CancellationToken.None);

        Assert.True(r.Success);
        Assert.InRange(r.Quality, 30, 95);
        Assert.True(r.Probes <= Compressor.ProbeBudget);
        Assert.True(r.Jpeg!.Length <= 30_000);
        Assert.Equal(4000, r.Width);
        Assert.Equal(3000, r.Height);
        // size(q) = 512 + 720*q: q=40 -> 29,312 (fits), q=41 -> 30,032 (does not).
        Assert.Equal(40, r.Quality);
        Assert.Equal(14, r.Probes); // 12 ladder probes + 2 bisect probes
    }

    [Fact] // AC-1.1: aspect preserved within 1 px
    public void Success_KeepsSourceAspectWithinOnePixel()
    {
        var image = TestImages.Noise(1600, 1200, 13);
        var codec = new FakeJpegCodec(image) { SizeFactor = 0.002 };

        FitResult r = Compressor.Fit(new byte[] { 1 }, codec, Settings(50), CancellationToken.None);

        Assert.True(r.Success);
        double impliedWidth = (double)r.Height * 1600 / 1200;
        Assert.True(Math.Abs(r.Width - impliedWidth) <= 1.0);
    }

    [Fact] // AC-2.1: both limits, 600x600 box
    public void BothLimits_OutputRespectsBoxAndAspect()
    {
        var image = TestImages.Noise(4000, 3000, 14);
        var codec = new FakeJpegCodec(image) { SizeFactor = 0.05 };

        FitResult r = Compressor.Fit(new byte[] { 1 }, codec, Settings(50, 600, 600), CancellationToken.None);

        Assert.True(r.Success);
        Assert.True(r.Width <= 600);
        Assert.True(r.Height <= 600);
        Assert.Equal(600, r.Width);
        Assert.Equal(450, r.Height);
        Assert.True(Math.Abs(r.Width - (double)r.Height * 4 / 3) <= 1.0);
        Assert.True(codec.Calls.All(c => c.W <= 600 && c.H <= 600));
    }

    [Fact] // AC-2.2: never upscale through the codec
    public void SmallImage_BiggerLimits_IsNotUpscaled()
    {
        var image = TestImages.Noise(400, 300, 15);
        var codec = new FakeJpegCodec(image);

        FitResult r = Compressor.Fit(new byte[] { 1 }, codec, Settings(500, 600, 600), CancellationToken.None);

        Assert.True(r.Success);
        Assert.Equal(400, r.Width);
        Assert.Equal(300, r.Height);
        Assert.All(codec.Calls, c => Assert.Equal((400, 300), (c.W, c.H)));
    }

    [Fact] // AC-6.1: unreachable target is stated, not faked
    public void UnreachableTarget_ReturnsTooSmallWithoutBytes()
    {
        var image = TestImages.Noise(4000, 3000, 16);
        var codec = new FakeJpegCodec(image) { Header = 6000, SizeFactor = 0.0001 };

        FitResult r = Compressor.Fit(new byte[] { 1 }, codec, Settings(5), CancellationToken.None);

        Assert.False(r.Success);
        Assert.Null(r.Jpeg);
        Assert.Equal(ErrorCodes.TooSmall, r.ErrorCode);
        Assert.StartsWith("Cannot reach 5 KB.", r.ErrorDetail);
        Assert.Contains("is 6", r.ErrorDetail); // best achievable size in KB
        Assert.Contains("×", r.ErrorDetail);
        Assert.True(r.BestBytes > 5000);
        Assert.True(r.Width >= 64 && r.Height >= 64);
    }

    [Fact] // AC-6.2: at most 8 downscales and never below 64 px
    public void DownscaleLoop_StopsAfterEightStepsAndNeverBelow64()
    {
        var image = TestImages.Noise(4000, 3000, 17);
        var codec = new FakeJpegCodec(image) { Header = 6000, SizeFactor = 0.0001 };

        FitResult r = Compressor.Fit(new byte[] { 1 }, codec, Settings(5), CancellationToken.None);

        Assert.False(r.Success);

        var dims = codec.Calls.Select(c => (c.W, c.H)).Distinct().ToList();
        Assert.Equal(9, dims.Count); // initial plan + 8 downscale steps
        Assert.All(dims, d => Assert.True(d.W >= 64 && d.H >= 64, $"{d.W}x{d.H} below floor"));

        // Height is always recomputed from the plan's aspect (4:3), not from the previous step.
        Assert.All(dims, d => Assert.True(Math.Abs(d.W - (double)d.H * 4 / 3) <= 1.0));
        Assert.All(dims.Skip(1), d => Assert.True(d.W < 4000));
    }

    [Fact]
    public void DownscaleLoop_KeepsAspectForPortrait()
    {
        var image = TestImages.Noise(3000, 4000, 18);
        var codec = new FakeJpegCodec(image) { Header = 6000, SizeFactor = 0.0001 };

        FitResult r = Compressor.Fit(new byte[] { 1 }, codec, Settings(5), CancellationToken.None);

        Assert.False(r.Success);
        Assert.All(codec.Calls, c => Assert.True(Math.Abs(c.H - (double)c.W * 4 / 3) <= 1.0));
    }

    [Fact] // AC-5.2: cancellation observed within one probe, no OCE escapes
    public void Cancellation_IsConvertedToCancelledResult()
    {
        var image = TestImages.Noise(1200, 900, 19);
        using var cts = new CancellationTokenSource();
        var codec = new FakeJpegCodec(image)
        {
            SizeFactor = 0.002,
            OnEncode = call => { if (call == 2) cts.Cancel(); }
        };

        FitResult r = Compressor.Fit(new byte[] { 1 }, codec, Settings(3), cts.Token);
        Assert.False(r.Success);
        Assert.Null(r.Jpeg);
        Assert.Equal(ErrorCodes.Cancelled, r.ErrorCode);
        Assert.True(codec.EncodeCalls <= 3, $"calls={codec.EncodeCalls}");
    }

    [Fact]
    public void CancelledBeforeStart_ReturnsCancelledResult()
    {
        var image = TestImages.Noise(100, 100, 20);
        var codec = new FakeJpegCodec(image);
        using var cts = new CancellationTokenSource();
        cts.Cancel();

        FitResult r = Compressor.Fit(new byte[] { 1 }, codec, Settings(10), cts.Token);

        Assert.False(r.Success);
        Assert.Equal(ErrorCodes.Cancelled, r.ErrorCode);
        Assert.Equal(0, codec.EncodeCalls);
    }

    [Fact] // AC-8.1 (core half): same input twice -> identical bytes
    public void SameInputTwice_ProducesByteIdenticalOutput()
    {
        var image = TestImages.Noise(800, 600, 21);
        byte[] input = { 5, 6, 7 };

        FitResult a = Compressor.Fit(input, new FakeJpegCodec(image) { SizeFactor = 0.01 }, Settings(20), CancellationToken.None);
        FitResult b = Compressor.Fit(input, new FakeJpegCodec(image) { SizeFactor = 0.01 }, Settings(20), CancellationToken.None);

        Assert.True(a.Success);
        Assert.True(b.Success);
        Assert.Equal(a.Quality, b.Quality);
        Assert.Equal(a.Width, b.Width);
        Assert.Equal(a.Height, b.Height);
        Assert.Equal(SHA256.HashData(a.Jpeg!), SHA256.HashData(b.Jpeg!));
    }

    [Fact]
    public void EmptyInput_IsEmptyError()
    {
        var codec = new FakeJpegCodec(TestImages.Noise(10, 10, 22));
        FitResult r = Compressor.Fit(Array.Empty<byte>(), codec, Settings(100), CancellationToken.None);

        Assert.False(r.Success);
        Assert.Equal(ErrorCodes.Empty, r.ErrorCode);
        Assert.Equal("Empty file.", r.ErrorDetail);
        Assert.Equal(0, codec.EncodeCalls);
    }

    [Fact]
    public void UnsupportedExtension_IsFormatError()
    {
        var codec = new FakeJpegCodec(TestImages.Noise(10, 10, 23));
        FitResult r = Compressor.Fit("photo.heic", new byte[] { 1 }, codec, Settings(100), CancellationToken.None);

        Assert.False(r.Success);
        Assert.Equal(ErrorCodes.Format, r.ErrorCode);
        Assert.Equal(ErrorMessages.Format, r.ErrorDetail);
    }

    [Fact]
    public void SupportedExtension_IsAccepted()
    {
        var codec = new FakeJpegCodec(TestImages.Noise(10, 10, 24));
        FitResult r = Compressor.Fit("photo.PNG", new byte[] { 1 }, codec, Settings(100, 600, 600), CancellationToken.None);
        Assert.Equal(ErrorCodes.Empty, "E_EMPTY"); // sanity: the code lives in ErrorCodes
        Assert.NotEqual(ErrorCodes.Format, r.ErrorCode);
    }

    [Fact]
    public void CorruptImage_IsDecodeError()
    {
        var codec = new FakeJpegCodec(TestImages.Noise(10, 10, 25)) { ThrowUnsupported = true };
        FitResult r = Compressor.Fit(new byte[] { 1 }, codec, Settings(100), CancellationToken.None);

        Assert.False(r.Success);
        Assert.Equal(ErrorCodes.Decode, r.ErrorCode);
        Assert.Equal(ErrorMessages.Decode, r.ErrorDetail);
    }

    [Fact]
    public void CodecCrash_IsDecodeErrorNotCrash()
    {
        var codec = new FakeJpegCodec(TestImages.Noise(10, 10, 26)) { ThrowGeneric = true };
        FitResult r = Compressor.Fit(new byte[] { 1 }, codec, Settings(100), CancellationToken.None);

        Assert.False(r.Success);
        Assert.Equal(ErrorCodes.Decode, r.ErrorCode);
    }

    [Fact]
    public void MismatchedPixelBuffer_IsDecodeError()
    {
        var broken = new Rgb24Image(100, 100, new byte[10]);
        var codec = new FakeJpegCodec(broken);
        FitResult r = Compressor.Fit(new byte[] { 1 }, codec, Settings(100), CancellationToken.None);

        Assert.False(r.Success);
        Assert.Equal(ErrorCodes.Decode, r.ErrorCode);
    }

    [Fact] // PRD §7 "other specified cases": 1x1 with a 1 KB target succeeds
    public void OnePixelImage_Succeeds()
    {
        var image = TestImages.Solid(1, 1, 10, 20, 30);
        var codec = new FakeJpegCodec(image);

        FitResult r = Compressor.Fit(new byte[] { 1 }, codec, Settings(1, 600, 600), CancellationToken.None);

        Assert.True(r.Success);
        Assert.Equal(1, r.Width);
        Assert.Equal(1, r.Height);
        Assert.Equal(95, r.Quality);
    }

    [Fact]
    public void QualityNeverLeavesThirtyToNinetyFive()
    {
        var image = TestImages.Noise(900, 700, 27);
        var codec = new FakeJpegCodec(image) { SizeFactor = 0.02 };

        Compressor.Fit(new byte[] { 1 }, codec, Settings(15), CancellationToken.None);

        Assert.NotEmpty(codec.Calls);
        Assert.All(codec.Calls, c => Assert.InRange(c.Q, 30, 95));
    }

    [Fact]
    public void SourceDpi_IsPreserved()
    {
        var image = TestImages.Noise(200, 200, 28);
        var codec = new FakeJpegCodec(image, new ImageMeta(300, 300, 1));

        Compressor.Fit(new byte[] { 1 }, codec, Settings(500), CancellationToken.None);

        Assert.All(codec.Calls, c => Assert.Equal(300, c.Dpi));
    }

    [Fact]
    public void InvalidDpi_FallsBackTo96()
    {
        var image = TestImages.Noise(200, 200, 29);
        var codec = new FakeJpegCodec(image, new ImageMeta(0, 5000, 1));

        Compressor.Fit(new byte[] { 1 }, codec, Settings(500), CancellationToken.None);

        Assert.All(codec.Calls, c => Assert.Equal(96, c.Dpi));
    }

    [Fact]
    public void OrientationIsBakedBeforePlanning() // AC-4.3 shape half
    {
        var image = TestImages.Noise(400, 300, 30);
        var codec = new FakeJpegCodec(image, new ImageMeta(96, 96, 6));

        FitResult r = Compressor.Fit(new byte[] { 1 }, codec, Settings(500, 600, 600), CancellationToken.None);

        Assert.True(r.Success);
        Assert.Equal(300, r.Width);
        Assert.Equal(400, r.Height);
        Assert.True(r.Width < r.Height);
    }

    [Fact]
    public void PortraitHeightLimit_IsApplied()
    {
        var image = TestImages.Noise(300, 400, 31);
        var codec = new FakeJpegCodec(image);

        FitResult r = Compressor.Fit(new byte[] { 1 }, codec, Settings(500, null, 200), CancellationToken.None);

        Assert.True(r.Success);
        Assert.Equal(150, r.Width);
        Assert.Equal(200, r.Height);
    }

    [Fact]
    public void ProbesStayWithinBudgetOnWorstCase()
    {
        var image = TestImages.Noise(4000, 3000, 32);
        var codec = new FakeJpegCodec(image) { Header = 6000, SizeFactor = 0.0001 };

        FitResult r = Compressor.Fit(new byte[] { 1 }, codec, Settings(5), CancellationToken.None);

        Assert.True(r.Probes <= Compressor.ProbeBudget, $"probes={r.Probes}");
        Assert.Equal(codec.EncodeCalls, r.Probes);
    }

    [Fact]
    public void SuccessReportsBytesAndNoError()
    {
        var image = TestImages.Noise(500, 400, 33);
        var codec = new FakeJpegCodec(image) { SizeFactor = 0.05 };

        FitResult r = Compressor.Fit(new byte[] { 1 }, codec, Settings(60), CancellationToken.None);

        Assert.True(r.Success);
        Assert.Equal(r.Jpeg!.LongLength, r.BestBytes);
        Assert.Null(r.ErrorCode);
        Assert.Null(r.ErrorDetail);
    }

    [Fact]
    public void NullArguments_Throw()
    {
        Assert.Throws<ArgumentNullException>(
            () => Compressor.Fit(new byte[] { 1 }, null!, Settings(10), CancellationToken.None));
        Assert.Throws<ArgumentNullException>(
            () => Compressor.Fit(new byte[] { 1 }, new FakeJpegCodec(TestImages.Noise(4, 4, 34)), null!, CancellationToken.None));
    }
}
