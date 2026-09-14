using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>A call record in a bubble: which is a missed call, and when and how it calls back.</summary>
public sealed class CallRecordsTests
{
    private static CallRecordDto Record(string outcome, bool video = false) => new(outcome, Video: video);

    [Fact]
    public void OnlyACallTheReaderMissedIsMarked()
    {
        Assert.True(CallRecords.IsMissed(Record("missed"), mine: false));
        Assert.False(CallRecords.IsMissed(Record("missed"), mine: true));
        Assert.False(CallRecords.IsMissed(Record("declined"), mine: false));
        Assert.False(CallRecords.IsMissed(Record("completed"), mine: false));
        Assert.False(CallRecords.IsMissed(Record("failed"), mine: false));
    }

    /// <summary>In a direct chat, on a server with calls, and not on a thread's panel — each of the three alone refuses it.</summary>
    [Fact]
    public void CallBackIsOfferedOnlyWhereACallCanBePlaced()
    {
        var voice = Record("missed");
        Assert.True(CallRecords.CallBack(voice, directChat: true, callsEnabled: true, videoCallsEnabled: true, inThread: false).Offered);
        Assert.False(CallRecords.CallBack(voice, directChat: false, callsEnabled: true, videoCallsEnabled: true, inThread: false).Offered);
        Assert.False(CallRecords.CallBack(voice, directChat: true, callsEnabled: false, videoCallsEnabled: true, inThread: false).Offered);
        Assert.False(CallRecords.CallBack(voice, directChat: true, callsEnabled: true, videoCallsEnabled: true, inThread: true).Offered);
    }

    /// <summary>A video record calls back with video while the server carries it, and with voice once it does not; a voice record never with video.</summary>
    [Fact]
    public void AVideoRecordCallsBackWithVideoOnlyWhereVideoIsCarried()
    {
        Assert.Equal((true, true), CallRecords.CallBack(Record("completed", video: true), true, true, videoCallsEnabled: true, false));
        Assert.Equal((true, false), CallRecords.CallBack(Record("completed", video: true), true, true, videoCallsEnabled: false, false));
        Assert.Equal((true, false), CallRecords.CallBack(Record("completed"), true, true, videoCallsEnabled: true, false));
    }
}
