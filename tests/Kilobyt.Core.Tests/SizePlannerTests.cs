using System;
using Kilobyt.Core;
using Xunit;

namespace Kilobyt.Core.Tests;

public class SizePlannerTests
{
    [Fact]
    public void NoLimits_ReturnsSource()
    {
        Assert.Equal((4000, 3000), SizePlanner.Fit(4000, 3000, null, null));
    }

    [Fact]
    public void WidthOnly_ScalesProportionally() // AC-2.3
    {
        Assert.Equal((600, 450), SizePlanner.Fit(4000, 3000, 600, null));
    }

    [Fact]
    public void LimitsAboveSource_DoNotUpscale() // AC-2.3
    {
        Assert.Equal((4000, 3000), SizePlanner.Fit(4000, 3000, 10000, 10000));
    }

    [Fact]
    public void SourceUnderLimits_StaysUnchanged() // AC-2.2
    {
        Assert.Equal((400, 300), SizePlanner.Fit(400, 300, 600, 600));
    }

    [Fact]
    public void BothLimits_UsesTighterOne()
    {
        Assert.Equal((600, 450), SizePlanner.Fit(4000, 3000, 600, 600));
    }

    [Fact]
    public void HeightIsTheTighterLimit()
    {
        Assert.Equal((200, 150), SizePlanner.Fit(4000, 3000, null, 150));
    }

    [Fact]
    public void Portrait_ScalesByHeight()
    {
        Assert.Equal((150, 200), SizePlanner.Fit(300, 400, 600, 200));
    }

    [Fact]
    public void OnePixelImage_Survives()
    {
        Assert.Equal((1, 1), SizePlanner.Fit(1, 1, null, null));
        Assert.Equal((1, 1), SizePlanner.Fit(1, 1, 600, 600));
    }

    [Fact]
    public void ExtremeLimit_ClampsToAtLeastOnePixel()
    {
        (int w, int h) = SizePlanner.Fit(4000, 3000, 1, null);
        Assert.Equal(1, w);
        Assert.True(h >= 1);
    }

    [Theory]
    [InlineData(4000, 3001, 999, null)]
    [InlineData(1024, 768, 600, 600)]
    [InlineData(3999, 2999, 1600, 1600)]
    [InlineData(1200, 799, 601, null)]
    [InlineData(777, 1001, null, 640)]
    public void AspectStaysWithinOnePixel(int w, int h, int? maxW, int? maxH)
    {
        (int nw, int nh) = SizePlanner.Fit(w, h, maxW, maxH);
        double sourceIdeal = (double)nh * w / h; // width implied by the destination height
        Assert.True(Math.Abs(nw - sourceIdeal) <= 1.0, $"aspect drift {nw} vs {sourceIdeal:0.###}");
        Assert.True(nw <= w && nh <= h);
    }

    [Fact]
    public void NonPositiveLimitsAreIgnored()
    {
        Assert.Equal((4000, 3000), SizePlanner.Fit(4000, 3000, 0, -5));
    }

    [Theory]
    [InlineData(0, 10)]
    [InlineData(10, 0)]
    [InlineData(-1, 10)]
    public void InvalidDimensionsThrow(int w, int h)
    {
        Assert.Throws<ArgumentOutOfRangeException>(() => SizePlanner.Fit(w, h, null, null));
    }
}
