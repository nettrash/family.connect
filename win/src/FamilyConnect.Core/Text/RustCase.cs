namespace FamilyConnect.Core;

/// <summary>
/// Rust's <c>char</c> case rules — <c>is_lowercase</c> and <c>to_uppercase</c> — for code outside this assembly. The web's
/// capital-letter rule (<c>fc_text::composer::capital_of</c>, issue #97) decides by them, and .NET's own are a different
/// Unicode version with SIMPLE case mapping: <c>char.ToUpperInvariant('ᾳ')</c> is one letter where Rust's full mapping makes
/// two, so a rule built on .NET would capitalise what the web leaves alone. The tables are <see cref="UnicodeProperties"/>,
/// printed by Rust itself.
/// </summary>
public static class RustCase
{
    /// <summary><c>char::is_lowercase</c>: the Lowercase property, which is more than the Ll category (<c>ª</c> is in it).</summary>
    public static bool IsLowercase(int scalar) => RustChar.IsLowercase(scalar);

    /// <summary>
    /// <c>char::to_uppercase</c> when that is exactly ONE scalar other than <paramref name="scalar"/> itself, or null: <c>ß</c>
    /// (upper case <c>SS</c>), <c>A</c> (already upper case) and <c>日</c> (no case) have none.
    /// </summary>
    public static int? SingleUppercase(int scalar)
    {
        using var upper = RustChar.Uppercase(scalar).GetEnumerator();
        if (!upper.MoveNext())
        {
            return null;
        }
        var first = upper.Current;
        return upper.MoveNext() || first == scalar ? null : first;
    }
}
