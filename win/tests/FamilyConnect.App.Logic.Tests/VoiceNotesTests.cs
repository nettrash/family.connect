using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>A voice note: five minutes at most, nothing when it is too short, and the lines it is drawn with.</summary>
public sealed class VoiceNotesTests
{
    [Fact]
    public void ARecordingIsDoneAtFiveMinutes()
    {
        Assert.False(VoiceNotes.IsDone(TimeSpan.FromSeconds(299.9)));
        Assert.True(VoiceNotes.IsDone(TimeSpan.FromMinutes(5)));
        Assert.True(VoiceNotes.IsDone(TimeSpan.FromMinutes(6)));
    }

    /// <summary>1024 bytes or less is nothing; past it, an MP4 audio attachment with no name, its duration its identity.</summary>
    [Fact]
    public void ARecordingIsStagedUnlessItIsNothing()
    {
        Assert.Null(VoiceNotes.Staged(new byte[1024], TimeSpan.FromSeconds(3)));
        Assert.Null(VoiceNotes.Staged(ReadOnlyMemory<byte>.Empty, TimeSpan.FromSeconds(3)));

        // Half a millisecond rounds away from zero, as the Apple apps' does — not to the even neighbour.
        var staged = VoiceNotes.Staged(new byte[1025], TimeSpan.FromMilliseconds(12_344.5))!;
        Assert.Equal(("audio", "audio/mp4", 1025, 12_345), (staged.Kind, staged.Mime, staged.Bytes.Length, staged.DurationMs));
        Assert.Null(staged.Name);
        Assert.Null(staged.Preview);

        // A recording that ran over by a tick is still five minutes long.
        Assert.Equal(300_000, VoiceNotes.Staged(new byte[4096], TimeSpan.FromSeconds(300.4))!.DurationMs);
    }

    [Fact]
    public void ARecordingSaysHowLongItHasRun()
    {
        var say = EnglishCatalog.Instance;
        Assert.Equal("Recording 0:12", VoiceNotes.RecordingLine(TimeSpan.FromSeconds(12.4), say));
        Assert.Equal("Recording 0:13", VoiceNotes.RecordingLine(TimeSpan.FromSeconds(12.5), say));
        Assert.Equal("Recording 5:00", VoiceNotes.RecordingLine(TimeSpan.FromMinutes(5), say));
    }

    /// <summary>The scrubber's length comes from the attachment, and is never zero.</summary>
    [Fact]
    public void ARecordingIsAsLongAsItsAttachmentSays()
    {
        Assert.Equal(65.0, VoiceNotes.TotalSeconds(65_000));
        Assert.Equal(1.5, VoiceNotes.TotalSeconds(1_500));
        Assert.Equal(0.1, VoiceNotes.TotalSeconds(null));
        Assert.Equal(0.1, VoiceNotes.TotalSeconds(0));
        Assert.Equal(0.1, VoiceNotes.TotalSeconds(40));
    }

    [Fact]
    public void PlayStartsAgainOnlyAtTheEnd()
    {
        Assert.False(VoiceNotes.ReplaysFromStart(0, 65));
        Assert.False(VoiceNotes.ReplaysFromStart(64.7, 65));
        Assert.True(VoiceNotes.ReplaysFromStart(64.8, 65));
        Assert.True(VoiceNotes.ReplaysFromStart(65, 65));
        Assert.True(VoiceNotes.ReplaysFromStart(66, 65));
    }
}
