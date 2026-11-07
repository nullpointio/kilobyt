using System;
using System.Collections.Generic;
using System.Threading;
using Kilobyt.Core;
using Xunit;

namespace Kilobyt.Core.Tests;

public class QualitySearchTests
{
    private static Func<int, long> Linear(long perUnit, List<int>? log = null)
        => q => { log?.Add(q); return q * perUnit; };

    [Fact]
    public void TargetAboveQ95_ReturnsMaximumQualityWithOneProbe()
    {
        var log = new List<int>();
        int? q = QualitySearch.Find(Linear(100, log), 100_000, CancellationToken.None);
        Assert.Equal(95, q);
        Assert.Equal(new[] { 95 }, log);
    }

    [Fact]
    public void FindsHighestPassingLadderStep()
    {
        // 10 bytes per quality unit: q=50 -> 500 <= 500 target.
        var log = new List<int>();
        int? q = QualitySearch.Find(Linear(10, log), 500, CancellationToken.None);
        Assert.Equal(50, q);
        Assert.Equal(new[] { 95, 90, 85, 80, 75, 70, 65, 60, 55, 50, 52, 51 }, log);
    }

    [Fact]
    public void BisectFindsHighestIntegerQuality()
    {
        // target 539: q=53 (530) passes, q=54 (540) does not.
        int? q = QualitySearch.Find(Linear(10), 539, CancellationToken.None);
        Assert.Equal(53, q);
    }

    [Fact]
    public void ReturnsNullWhenEvenQ30IsTooLarge()
    {
        var log = new List<int>();
        int? q = QualitySearch.Find(Linear(10, log), 100, CancellationToken.None);
        Assert.Null(q);
        Assert.Equal(14, log.Count); // the whole ladder, no bisect
        Assert.Equal(30, log[^1]);
    }

    [Fact]
    public void LadderNeverProbesOutsideThirtyToNinetyFive()
    {
        var log = new List<int>();
        QualitySearch.Find(Linear(10, log), 640, CancellationToken.None);
        Assert.All(log, q => Assert.InRange(q, 30, 95));
    }

    [Fact]
    public void BisectIsCappedAtFourExtraProbes()
    {
        var log = new List<int>();
        int? q = QualitySearch.Find(Linear(10, log), 490, CancellationToken.None);
        Assert.Equal(49, q);
        Assert.True(log.Count <= 14 + 4, $"probes={log.Count}");
    }

    [Fact]
    public void CancelledBeforeStart_ThrowsWithoutProbing()
    {
        using var cts = new CancellationTokenSource();
        cts.Cancel();
        var log = new List<int>();
        Assert.Throws<OperationCanceledException>(
            () => QualitySearch.Find(Linear(10, log), 500, cts.Token));
        Assert.Empty(log);
    }

    [Fact]
    public void CancellationObservedBeforeNextProbe()
    {
        using var cts = new CancellationTokenSource();
        var log = new List<int>();
        Assert.Throws<OperationCanceledException>(() => QualitySearch.Find(
            q =>
            {
                log.Add(q);
                if (log.Count == 2) cts.Cancel();
                return q * 10L;
            },
            500,
            cts.Token));
        Assert.Equal(2, log.Count); // no probe after the cancel was requested
    }

    [Fact]
    public void ProbeIsRequired()
    {
        Assert.Throws<ArgumentNullException>(
            () => QualitySearch.Find(null!, 100, CancellationToken.None));
    }
}
