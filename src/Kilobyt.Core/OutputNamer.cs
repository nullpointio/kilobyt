namespace Kilobyt.Core;

/// <summary>
/// PRD §5.6 — <c>{stem}_fit.jpg</c>, then <c>-2</c> … <c>-999</c>. Never overwrites.
/// The returned path is added to <c>reserved</c> (when supplied) so a batch cannot hand
/// out the same name twice; callers may also pre-add paths.
/// </summary>
public static class OutputNamer
{
    public const int MaxSuffix = 999;

    public static string NextFree(string dir, string stem, string ext, ISet<string> reserved)
    {
        if (dir is null) throw new ArgumentNullException(nameof(dir));
        if (stem is null) throw new ArgumentNullException(nameof(stem));
        if (ext is null) throw new ArgumentNullException(nameof(ext));

        string cleanStem = SanitiseStem(stem);
        string cleanExt = NormaliseExtension(ext);
        string baseName = cleanStem + "_fit";

        reserved ??= new HashSet<string>(StringComparer.OrdinalIgnoreCase);

        for (int i = 1; i <= MaxSuffix; i++)
        {
            string name = i == 1 ? baseName + cleanExt : $"{baseName}-{i}{cleanExt}";
            string candidate = Path.Combine(dir, name);
            if (Exists(candidate, reserved))
            {
                continue;
            }

            reserved.Add(candidate);
            return candidate;
        }

        throw new NameCollisionException(ErrorMessages.Name(cleanStem));
    }

    /// <summary>Windows rule: a file name may not end in dots or spaces.</summary>
    public static string SanitiseStem(string stem)
    {
        string s = stem.TrimEnd('.', ' ');
        return s.Length == 0 ? "image" : s;
    }

    /// <summary>Accepts "jpg" or ".jpg" and lower-cases the extension.</summary>
    public static string NormaliseExtension(string ext)
    {
        string e = ext.Trim();
        if (e.Length == 0) throw new ArgumentException("Extension must not be empty.", nameof(ext));
        if (!e.StartsWith('.')) e = "." + e;
        return e.ToLowerInvariant();
    }

    private static bool Exists(string candidate, ISet<string> reserved)
    {
        try
        {
            if (File.Exists(candidate)) return true;
            if (Directory.Exists(candidate)) return true;
        }
        catch (IOException)
        {
            // Treat an unreadable probe as taken: never risk an overwrite.
            return true;
        }
        catch (UnauthorizedAccessException)
        {
            return true;
        }

        if (reserved.Count == 0) return false;
        if (reserved.Contains(candidate)) return true;

        // Reserved sets may hold either the combined path or just the file name.
        string full;
        try { full = Path.GetFullPath(candidate); }
        catch (Exception) { full = candidate; }

        foreach (string r in reserved)
        {
            if (string.Equals(r, candidate, StringComparison.OrdinalIgnoreCase)) return true;
            string other;
            try { other = Path.GetFullPath(r); }
            catch (Exception) { other = r; }
            if (string.Equals(other, full, StringComparison.OrdinalIgnoreCase)) return true;
        }

        return false;
    }
}
