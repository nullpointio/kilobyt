using System;
using System.Collections.Generic;
using System.IO;
using Kilobyt.Core;
using Xunit;

namespace Kilobyt.Core.Tests;

public class OutputNamerTests
{
    private static string TempDir()
    {
        string dir = Path.Combine(Path.GetTempPath(), "kilobyt-namer-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(dir);
        return dir;
    }

    [Fact]
    public void FirstFree_UsesFitSuffix()
    {
        string dir = TempDir();
        string path = OutputNamer.NextFree(dir, "IMG_2031", ".jpg", new HashSet<string>());
        Assert.Equal(Path.Combine(dir, "IMG_2031_fit.jpg"), path);
    }

    [Fact]
    public void ExistingFile_BecomesDashTwo()
    {
        string dir = TempDir();
        File.WriteAllBytes(Path.Combine(dir, "a_fit.jpg"), new byte[] { 1 });

        string path = OutputNamer.NextFree(dir, "a", ".jpg", new HashSet<string>());
        Assert.Equal(Path.Combine(dir, "a_fit-2.jpg"), path);
    }

    [Fact]
    public void ExistingFiles_WalkUpTheSuffixes()
    {
        string dir = TempDir();
        File.WriteAllBytes(Path.Combine(dir, "a_fit.jpg"), new byte[] { 1 });
        File.WriteAllBytes(Path.Combine(dir, "a_fit-2.jpg"), new byte[] { 1 });
        File.WriteAllBytes(Path.Combine(dir, "a_fit-3.jpg"), new byte[] { 1 });

        string path = OutputNamer.NextFree(dir, "a", ".jpg", new HashSet<string>());
        Assert.Equal(Path.Combine(dir, "a_fit-4.jpg"), path);
    }

    [Fact]
    public void ReservedName_IsSkippedWithoutDisk()
    {
        string dir = Path.Combine(Path.GetTempPath(), "kilobyt-gone-" + Guid.NewGuid().ToString("N"));
        var reserved = new HashSet<string>(StringComparer.OrdinalIgnoreCase)
        {
            Path.Combine(dir, "a_fit.jpg")
        };
        // The directory does not exist: only the reserved set can collide.
        string path = OutputNamer.NextFree(dir, "a", ".jpg", reserved);
        Assert.Equal(Path.Combine(dir, "a_fit-2.jpg"), path);
    }

    [Fact]
    public void SecondCallWithSameReservedSet_GetsDashTwo()
    {
        string dir = TempDir();
        var reserved = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        string first = OutputNamer.NextFree(dir, "a", ".jpg", reserved);
        string second = OutputNamer.NextFree(dir, "a", ".jpg", reserved);
        Assert.Equal(Path.Combine(dir, "a_fit.jpg"), first);
        Assert.Equal(Path.Combine(dir, "a_fit-2.jpg"), second);
    }

    [Fact]
    public void NeverOverwritesExistingContent()
    {
        string dir = TempDir();
        string existing = Path.Combine(dir, "a_fit.jpg");
        File.WriteAllBytes(existing, new byte[] { 42 });
        OutputNamer.NextFree(dir, "a", ".jpg", new HashSet<string>());
        Assert.Equal(new byte[] { 42 }, File.ReadAllBytes(existing));
    }

    [Fact]
    public void Exhausted999Collisions_ThrowsNameError()
    {
        string dir = @"X:\out";
        var reserved = new HashSet<string>(StringComparer.OrdinalIgnoreCase) { Path.Combine(dir, "a_fit.jpg") };
        for (int i = 2; i <= 999; i++)
            reserved.Add(Path.Combine(dir, $"a_fit-{i}.jpg"));

        var ex = Assert.Throws<NameCollisionException>(
            () => OutputNamer.NextFree(dir, "a", ".jpg", reserved));
        Assert.Contains("rename the source files", ex.Message);
        Assert.Contains("a_fit", ex.Message);
    }

    [Fact]
    public void Suffix999IsStillAllowed()
    {
        string dir = @"X:\out";
        var reserved = new HashSet<string>(StringComparer.OrdinalIgnoreCase) { Path.Combine(dir, "a_fit.jpg") };
        for (int i = 2; i <= 998; i++)
            reserved.Add(Path.Combine(dir, $"a_fit-{i}.jpg"));

        string path = OutputNamer.NextFree(dir, "a", ".jpg", reserved);
        Assert.EndsWith("a_fit-999.jpg", path);
    }

    [Fact]
    public void StemTrailingDotsAndSpaces_AreTrimmed()
    {
        string dir = @"X:\out";
        string path = OutputNamer.NextFree(dir, "report . .", ".jpg", new HashSet<string>());
        Assert.Equal(Path.Combine(dir, "report_fit.jpg"), path);
    }

    [Fact]
    public void ExtensionWithoutDot_IsNormalised()
    {
        string dir = @"X:\out";
        string path = OutputNamer.NextFree(dir, "a", "JPG", new HashSet<string>());
        Assert.Equal(Path.Combine(dir, "a_fit.jpg"), path);
    }

    [Fact]
    public void NullReservedSet_IsTolerated()
    {
        string dir = @"X:\out";
        string path = OutputNamer.NextFree(dir, "a", ".jpg", null!);
        Assert.Equal(Path.Combine(dir, "a_fit.jpg"), path);
    }

    [Fact]
    public void UnicodeStem_IsPreserved()
    {
        string dir = @"X:\out";
        string path = OutputNamer.NextFree(dir, "写真 😀", ".jpg", new HashSet<string>());
        Assert.Equal(Path.Combine(dir, "写真 😀_fit.jpg"), path);
    }
}
