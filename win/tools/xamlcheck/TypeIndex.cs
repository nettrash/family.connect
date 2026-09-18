using System.Reflection;

namespace XamlCheck;

/// <summary>A XAML element's CLR type: from the referenced assemblies (Metadata set) or declared by the app itself (Metadata null).</summary>
sealed record XamlType(string FullName, Type? Metadata);

/// <summary>
/// Every public top-level type of the shadow project's compile references, read with
/// MetadataLoadContext so nothing WinRT is loaded for execution. Member lookups walk the base
/// chain and tolerate a base type whose assembly was not among the references.
/// </summary>
sealed class TypeIndex : IDisposable
{
    // The default XAML xmlns covers all Microsoft.UI.Xaml.* namespaces; when a simple name exists
    // in several of them this is the order tried.
    static readonly string[] Preference =
    [
        "Microsoft.UI.Xaml.Controls", "Microsoft.UI.Xaml", "Microsoft.UI.Xaml.Controls.Primitives",
        "Microsoft.UI.Xaml.Media", "Microsoft.UI.Xaml.Media.Animation", "Microsoft.UI.Xaml.Media.Imaging",
        "Microsoft.UI.Xaml.Shapes", "Microsoft.UI.Xaml.Documents", "Microsoft.UI.Xaml.Input",
        "Microsoft.UI.Xaml.Data", "Microsoft.UI.Xaml.Automation", "Microsoft.UI.Xaml.Navigation",
        "Microsoft.UI.Xaml.Markup", "Microsoft.UI.Xaml.Interop",
    ];
    // WinRT value types the default xmlns also reaches (<Color>, <Point>, <FontWeight>); Thickness and CornerRadius live in WinUI itself.
    static readonly string[] Fallback = ["Windows.UI", "Windows.Foundation", "Microsoft.UI", "Windows.UI.Text", "Microsoft.UI.Text"];

    const BindingFlags Declared = BindingFlags.Public | BindingFlags.DeclaredOnly;

    readonly MetadataLoadContext _mlc;
    readonly Dictionary<string, Type> _byFullName = new(StringComparer.Ordinal);
    readonly Dictionary<string, List<Type>> _bySimpleName = new(StringComparer.Ordinal);

    public int AssemblyCount { get; private set; }
    public int TypeCount => _byFullName.Count;

    TypeIndex(MetadataLoadContext mlc) => _mlc = mlc;

    public static TypeIndex Load(IReadOnlyList<string> paths, Log log)
    {
        // One path per simple name: PathAssemblyResolver rejects duplicates. First one wins.
        var unique = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        foreach (var p in paths) unique.TryAdd(Path.GetFileNameWithoutExtension(p), p);
        // The reference pack defines System.Object in System.Runtime; a runtime directory defines it in System.Private.CoreLib.
        var core = unique.ContainsKey("System.Private.CoreLib") ? "System.Private.CoreLib" : "System.Runtime";
        var index = new TypeIndex(new MetadataLoadContext(new PathAssemblyResolver(unique.Values), core));
        foreach (var p in unique.Values)
        {
            try
            {
                var asm = index._mlc.LoadFromAssemblyPath(p);
                foreach (var t in asm.GetExportedTypes())
                {
                    if (t.IsNested || t.FullName is null) continue;
                    index._byFullName.TryAdd(t.FullName, t);
                    if (!index._bySimpleName.TryGetValue(t.Name, out var list)) index._bySimpleName[t.Name] = list = [];
                    list.Add(t);
                }
                index.AssemblyCount++;
            }
            catch (Exception e)
            {
                log.Warn($"metadata: could not read {p}: {e.Message}");
            }
        }
        return index;
    }

    public Type? Find(string fullName) => _byFullName.GetValueOrDefault(fullName);

    /// <summary>Resolve an unprefixed element name the way the default xmlns does.</summary>
    public Type? FindInDefaultNamespace(string name, out bool ambiguous)
    {
        ambiguous = false;
        if (!_bySimpleName.TryGetValue(name, out var all)) return null;
        var hits = all.Where(t => IsXamlNamespace(t.Namespace)).ToList();
        if (hits.Count == 0) hits = all.Where(t => t.Namespace is not null && Array.IndexOf(Fallback, t.Namespace) >= 0).ToList();
        if (hits.Count == 0) return null;
        ambiguous = hits.Count > 1;
        return hits.OrderBy(t => Rank(t.Namespace)).ThenBy(t => t.FullName, StringComparer.Ordinal).First();
    }

    static bool IsXamlNamespace(string? ns) =>
        ns is not null && (ns == "Microsoft.UI.Xaml" || ns.StartsWith("Microsoft.UI.Xaml.", StringComparison.Ordinal));

    static int Rank(string? ns)
    {
        var i = Array.IndexOf(Preference, ns ?? "");
        return i < 0 ? int.MaxValue : i;
    }

    public IEnumerable<Type> BaseChain(Type type)
    {
        Type? current = type;
        while (current is not null)
        {
            yield return current;
            Type? next;
            try { next = current.BaseType; }
            catch (Exception) { next = null; } // the base lives in an assembly we were not given
            current = next;
        }
    }

    public bool DerivesFrom(Type type, string baseFullName) => BaseChain(type).Any(t => t.FullName == baseFullName);

    public bool HasInstanceProperty(Type type, string name) =>
        BaseChain(type).Any(t => Safe(() => t.GetProperties(Declared | BindingFlags.Instance)).Any(p => p.Name == name));

    public bool HasEvent(Type type, string name) =>
        BaseChain(type).Any(t => Safe(() => t.GetEvents(Declared | BindingFlags.Instance)).Any(e => e.Name == name));

    /// <summary>Attached property: the C#/WinRT projection exposes a static <c>XProperty</c> (a property, occasionally a field) and static <c>GetX</c>/<c>SetX</c>.</summary>
    public bool HasAttachedProperty(Type owner, string name) => BaseChain(owner).Any(t =>
        Safe(() => t.GetProperties(Declared | BindingFlags.Static)).Any(p => p.Name == name + "Property")
        || Safe(() => t.GetFields(Declared | BindingFlags.Static)).Any(f => f.Name == name + "Property")
        || Safe(() => t.GetMethods(Declared | BindingFlags.Static)).Any(m => m.Name == "Get" + name));

    static T[] Safe<T>(Func<T[]> read)
    {
        try { return read(); }
        catch (Exception) { return []; }
    }

    public void Dispose() => _mlc.Dispose();
}
