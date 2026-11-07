using Kilobyt.Core;
using Xunit;

namespace Kilobyt.Core.Tests;

public class FormatLimitsTests
{
    [Theory]
    [InlineData("photo.jpg")]
    [InlineData("photo.JPG")]
    [InlineData("photo.jpeg")]
    [InlineData("photo.png")]
    [InlineData("photo.bmp")]
    public void SupportedExtensions(string name)
    {
        Assert.True(FormatLimits.IsSupportedExtension(name));
        Assert.Null(FormatLimits.Validate(name, 1000));
    }

    [Theory]
    [InlineData("photo.heic")]
    [InlineData("photo.tiff")]
    [InlineData("photo.webp")]
    [InlineData("photo.gif")]
    [InlineData("photo")]
    [InlineData("")]
    public void UnsupportedExtensions_AreFormatErrors(string name)
    {
        Assert.False(FormatLimits.IsSupportedExtension(name));
        FitResult result = Assert.IsType<FitResult>(FormatLimits.Validate(name, 1000));
        Assert.Equal(ErrorCodes.Format, result.ErrorCode);
        Assert.Equal(ErrorMessages.Format, result.ErrorDetail);
    }

    [Fact]
    public void NullFileNameMeansUnknownName_NoFormatRule()
    {
        // A null name means the caller has no path yet (stream input); the extension rule
        // is then the shell's business, not a format failure.
        Assert.False(FormatLimits.IsSupportedExtension(null));
        Assert.Null(FormatLimits.Validate(null, 1000));
    }

    [Fact]
    public void ZeroLength_IsEmptyError()
    {
        FitResult result = Assert.IsType<FitResult>(FormatLimits.Validate("a.jpg", 0));
        Assert.Equal(ErrorCodes.Empty, result.ErrorCode);
        Assert.Equal("Empty file.", result.ErrorDetail);
    }

    [Fact]
    public void Over200Megabytes_IsTooBig()
    {
        FitResult result = Assert.IsType<FitResult>(
            FormatLimits.Validate("a.jpg", FormatLimits.MaxFileBytes + 1));
        Assert.Equal(ErrorCodes.TooBig, result.ErrorCode);
        Assert.Contains("200 MB", result.ErrorDetail);
        Assert.StartsWith("Too large:", result.ErrorDetail);
    }

    [Fact]
    public void Exactly200MegabytesIsAccepted()
    {
        Assert.Null(FormatLimits.Validate("a.jpg", FormatLimits.MaxFileBytes));
    }

    [Fact]
    public void Over80Megapixels_IsTooBigWithActualNumbers()
    {
        FitResult result = Assert.IsType<FitResult>(
            FormatLimits.Validate("a.jpg", 1000, 210_000_000));
        Assert.Equal(ErrorCodes.TooBig, result.ErrorCode);
        Assert.Equal("Too large: 210 MP. Limit is 80 MP.", result.ErrorDetail);
    }

    [Fact]
    public void Exactly80MegapixelsIsAccepted()
    {
        Assert.Null(FormatLimits.Validate("a.jpg", 1000, FormatLimits.MaxPixels));
    }

    [Fact]
    public void ConstantsMatchPrd()
    {
        Assert.Equal(200_000_000L, FormatLimits.MaxFileBytes);
        Assert.Equal(80_000_000L, FormatLimits.MaxPixels);
        Assert.Equal(200, Compressor.ProbeBudget);
    }
}
