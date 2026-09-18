using FamilyConnect.Core;
using Windows.Devices.Geolocation;
using Windows.Foundation;

namespace FamilyConnect.App.Services;

/// <summary>What the person has decided about location — settled BEFORE anything on screen says it is busy (#41).</summary>
internal enum LocationPermission
{
    /// <summary>A hunt may start.</summary>
    Allowed,

    /// <summary>Refused, now or earlier: the one answer worth pointing at Settings for.</summary>
    Denied,

    /// <summary>Nothing came back. Nothing was taken and nothing runs, so asking again is one click, not a failure to report.</summary>
    Unanswered,

    /// <summary>Windows could not be asked at all.</summary>
    Failed,
}

/// <summary>
/// One fix, on demand — the whole of what sharing a place needs (ios <c>LocationProvider</c>, web <c>location.rs</c>).
/// Deliberately NOT a running location service: it starts, waits for a fix good enough to send, and stops.
/// </summary>
/// <remarks>
/// <b>HOWEVER THE WAIT ENDS — A FIX, A REFUSAL, THE DEADLINE, OR THE CALLER GOING — THE GEOLOCATOR IS LET GO OF.</b> Its
/// events are what keep it sampling, and they are taken off in one place.
/// </remarks>
internal static class LocationFinder
{
    /// <summary>Ask, raising Windows's prompt if it has never been answered, and wait for the ANSWER — never for a clock.</summary>
    public static async Task<LocationPermission> RequestPermissionAsync()
    {
        try
        {
            return await Geolocator.RequestAccessAsync() switch
            {
                GeolocationAccessStatus.Allowed => LocationPermission.Allowed,
                GeolocationAccessStatus.Denied => LocationPermission.Denied,
                _ => LocationPermission.Unanswered,
            };
        }
        catch (Exception e)
        {
            Diagnostics.Write($"asking for location: {e.GetType().Name}");
            return LocationPermission.Failed;
        }
    }

    /// <summary>
    /// One fix from a device already allowed to give one: the first good enough, or at the deadline the best fresh one
    /// held — or null, with <c>Denied</c> when location was switched off while it looked.
    /// </summary>
    public static async Task<(LocationFix? Fix, bool Denied)> CurrentFixAsync(CancellationToken ct)
    {
        var hunt = new LocationHunt();
        var answered = new TaskCompletionSource<(LocationFix? Fix, bool Denied)>(TaskCreationOptions.RunContinuationsAsynchronously);
        var locator = new Geolocator
        {
            DesiredAccuracyInMeters = (uint)LocationFixes.GoodEnoughMetres,
            ReportInterval = 1000,
        };
        var changed = new TypedEventHandler<Geolocator, PositionChangedEventArgs>((_, e) =>
        {
            var coordinate = e.Position.Coordinate;
            var position = coordinate.Point.Position;
            var fix = new LocationFix(
                position.Latitude, position.Longitude, LocationFixes.UsableAccuracy(coordinate.Accuracy), coordinate.Timestamp);
            if (hunt.Offer(fix, DateTimeOffset.UtcNow) is { } good)
            {
                answered.TrySetResult((good, false));
            }
        });
        var status = new TypedEventHandler<Geolocator, StatusChangedEventArgs>((_, e) =>
        {
            // Switched off in Settings while it looked.
            if (e.Status == PositionStatus.Disabled)
            {
                answered.TrySetResult((null, true));
            }
        });
        locator.StatusChanged += status;
        locator.PositionChanged += changed;
        try
        {
            using var clock = CancellationTokenSource.CreateLinkedTokenSource(ct);
            var deadline = Task.Delay(LocationFixes.Timeout, clock.Token);
            var first = await Task.WhenAny(answered.Task, deadline);
            clock.Cancel();
            if (first == answered.Task)
            {
                return await answered.Task;
            }
            // A coarse fix beats no fix: whatever fresh one arrived while this held out for better goes now.
            return (ct.IsCancellationRequested ? null : hunt.Best, false);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"finding the location: {e.GetType().Name}");
            return (null, false);
        }
        finally
        {
            locator.PositionChanged -= changed;
            locator.StatusChanged -= status;
        }
    }
}
