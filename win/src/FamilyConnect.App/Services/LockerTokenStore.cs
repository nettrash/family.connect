using FamilyConnect.Core.Protocol;
using Windows.Security.Credentials;

namespace FamilyConnect.App.Services;

/// <summary>
/// The session token, in the Windows credential locker — the Keychain's and the Android Keystore's
/// counterpart. Core holds only the interface, because the secret is a Windows API's business.
/// </summary>
/// <remarks>
/// <para>
/// <b>ONE CREDENTIAL PER SERVER.</b> The account name is the server's origin, so pointing the app at a
/// second server neither hands it the first one's token nor forgets it.
/// </para>
/// <para>
/// <b>READ ONCE.</b> The client reads the token on every request, and the locker is not a field: it is
/// asked once and remembered, and every write goes to both.
/// </para>
/// </remarks>
internal sealed class LockerTokenStore(Uri server) : ITokenStore
{
    private const string Resource = "Family Connect";

    private readonly string account = server.GetLeftPart(UriPartial.Authority);
    private readonly object gate = new();
    private string? token;
    private bool loaded;

    public string? Token
    {
        get
        {
            lock (gate)
            {
                if (!loaded)
                {
                    token = Load();
                    loaded = true;
                }
                return token;
            }
        }
        set
        {
            lock (gate)
            {
                token = value is { Length: > 0 } ? value : null;
                loaded = true;
                Save(token);
            }
        }
    }

    private string? Load()
    {
        try
        {
            // Retrieve THROWS when there is nothing stored ("Element not found"), which is the
            // ordinary first-run state and not an error worth logging.
            var credential = new PasswordVault().Retrieve(Resource, account);
            credential.RetrievePassword();
            return credential.Password;
        }
        catch (Exception)
        {
            return null;
        }
    }

    private void Save(string? value)
    {
        try
        {
            var vault = new PasswordVault();
            try
            {
                foreach (var existing in vault.FindAllByResource(Resource))
                {
                    if (existing.UserName == account)
                    {
                        vault.Remove(existing);
                    }
                }
            }
            catch (Exception)
            {
                // FindAllByResource throws when the resource holds nothing yet.
            }
            if (value is not null)
            {
                vault.Add(new PasswordCredential(Resource, account, value));
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"credential locker: {e.GetType().Name}");
        }
    }
}
