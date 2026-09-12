namespace FamilyConnect.Core.Protocol;

/// <summary>
/// What a call answered: a value, or the reason there is none.
/// </summary>
/// <remarks>
/// A result rather than an exception, because on this wire a refusal is an ORDINARY answer that
/// the caller has to read — <see cref="ApiError.Transient"/> decides whether to try again, and an
/// exception thrown for a 429 would lose that distinction on the way up (docs/protocol.md,
/// "Error shape").
/// </remarks>
public readonly record struct ApiResult<T>(T? Value, ApiError? Error)
{
    public bool Ok => Error is null;

    public static ApiResult<T> Success(T value) => new(value, null);

    public static ApiResult<T> Failure(ApiError error) => new(default, error);

    /// <summary>The value, or <paramref name="fallback"/> when the call failed.</summary>
    public T? Or(T? fallback) => Ok ? Value : fallback;
}

/// <summary>An answer with no body — a <c>204</c>.</summary>
public readonly record struct Nothing
{
    public static readonly Nothing Value = new();
}

/// <summary>
/// Where the session token lives. Core holds the INTERFACE and never the secret: on Windows the
/// token belongs in the credential locker, which is a Windows API, and Core builds and tests
/// anywhere.
/// </summary>
public interface ITokenStore
{
    /// <summary>The token, or null when nobody is signed in.</summary>
    string? Token { get; set; }
}

/// <summary>For tests and for the moment before a real store is wired up.</summary>
public sealed class MemoryTokenStore(string? token = null) : ITokenStore
{
    public string? Token { get; set; } = token;
}
