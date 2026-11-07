namespace Kilobyt.Core;

/// <summary>
/// PRD §5.2.6 — coarse ladder 95→30 step 5, then integer bisect inside (q_hi, q_hi+5)
/// for the highest passing quality. Greedy-maximal over the probes actually performed.
/// </summary>
public static class QualitySearch
{
    public const int MinQuality = 30;
    public const int MaxQuality = 95;
    public const int LadderStep = 5;

    /// <summary>
    /// Returns the highest quality whose probed size is ≤ <paramref name="targetBytes"/>,
    /// or null when even q=30 is too large. Cancellation is observed before every probe.
    /// </summary>
    public static int? Find(Func<int, long> probe, long targetBytes, CancellationToken ct)
    {
        ArgumentNullException.ThrowIfNull(probe);

        int? qHi = null;
        for (int q = MaxQuality; q >= MinQuality; q -= LadderStep)
        {
            ct.ThrowIfCancellationRequested();
            if (probe(q) <= targetBytes)
            {
                qHi = q;
                break;
            }
        }

        if (qHi is null) return null;

        int best = qHi.Value;
        int lo = qHi.Value + 1;
        int hi = Math.Min(MaxQuality, qHi.Value + LadderStep - 1);
        while (lo <= hi)
        {
            int mid = lo + ((hi - lo) / 2);
            ct.ThrowIfCancellationRequested();
            if (probe(mid) <= targetBytes)
            {
                best = mid;
                lo = mid + 1;
            }
            else
            {
                hi = mid - 1;
            }
        }

        return best;
    }
}
