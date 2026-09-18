using System.Text.RegularExpressions;
using System.Xml;
using System.Xml.Linq;

namespace XamlCheck;

sealed record Diagnostic(bool IsError, string File, int Line, string Message)
{
    public string Format(string root) => $"{Path.GetRelativePath(root, File)}:{Line}: {(IsError ? "error" : "warning")}: {Message}";
}

sealed record XamlField(string Name, string TypeFullName, string Modifier, int Line);

/// <summary>One x:Class: what the XAML compiler would generate a partial for.</summary>
sealed class XamlClass
{
    public required string FullName { get; init; }
    public required string File { get; init; }
    public string Namespace => FullName.Contains('.') ? FullName[..FullName.LastIndexOf('.')] : "";
    public string Name => FullName[(FullName.LastIndexOf('.') + 1)..];
    public string? BaseTypeFullName { get; set; }
    public List<XamlField> Fields { get; } = [];
    public bool WantsConnect { get; set; }
}

sealed class LintResult
{
    public List<Diagnostic> Diagnostics { get; } = [];
    public List<XamlClass> Classes { get; } = [];
    public int FileCount { get; set; }
}

/// <summary>
/// Walks every *.xaml under the app directory and checks what the XAML compiler would check
/// first: that each element is a type, each attribute a property, event or attached property,
/// each event handler a method in the code-behind, and each x:Class a partial class. Collects
/// the x:Name fields for <see cref="StubWriter"/>.
/// </summary>
sealed class XamlLint(TypeIndex types, SourceIndex sources, Options options, Log log)
{
    static readonly XNamespace Presentation = "http://schemas.microsoft.com/winfx/2006/xaml/presentation";
    static readonly XNamespace X = "http://schemas.microsoft.com/winfx/2006/xaml";
    static readonly XNamespace Mc = "http://schemas.openxmlformats.org/markup-compatibility/2006";
    static readonly XNamespace Blend = "http://schemas.microsoft.com/expression/blend/2008";
    static readonly XNamespace Xml = "http://www.w3.org/XML/1998/namespace";
    const string Using = "using:";
    const string FrameworkTemplate = "Microsoft.UI.Xaml.FrameworkTemplate";
    const string FrameworkElement = "Microsoft.UI.Xaml.FrameworkElement";

    static readonly Regex XBind = new(@"\{\s*x:Bind\b", RegexOptions.Compiled);
    static readonly Regex Identifier = new(@"^[A-Za-z_][A-Za-z0-9_]*$", RegexOptions.Compiled);
    static readonly Regex QualifiedIdentifier = new(@"^[A-Za-z_][A-Za-z0-9_]*(\.[A-Za-z_][A-Za-z0-9_]*)*$", RegexOptions.Compiled);

    // x: elements the XAML compiler maps straight onto System types.
    static readonly Dictionary<string, string> XTypes = new()
    {
        ["String"] = "System.String", ["Double"] = "System.Double", ["Int32"] = "System.Int32",
        ["Boolean"] = "System.Boolean", ["Object"] = "System.Object", ["Uri"] = "System.Uri",
    };
    static readonly HashSet<string> XAttributes =
        ["Class", "Name", "Key", "Uid", "FieldModifier", "ClassModifier", "Load", "DeferLoadStrategy", "Phase", "DataType"];

    // Markup extensions the XAML parser implements itself and also accepts in object-element form:
    // <StaticResource x:Key="Alias" ResourceKey="Original" /> is how WinUI's own generic.xaml aliases
    // one resource to another (and the only way to alias a Color, which has no attribute to hang a
    // {ThemeResource} on). No WinRT type backs them, so they are matched by name; the value lists
    // the attributes they take besides x:Key.
    static readonly Dictionary<string, HashSet<string>> IntrinsicElements = new()
    {
        ["StaticResource"] = ["ResourceKey"],
    };

    readonly LintResult _result = new();

    // Per-file state.
    string _file = "";
    XamlClass? _class;
    HashSet<XNamespace> _ignored = [];
    HashSet<string> _reportedNamespaces = [];
    HashSet<string> _names = [];
    IReadOnlyList<SourceIndex.SourceFile> _codeBehind = [];
    IReadOnlyList<SourceIndex.SourceFile> _handlerScope = [];

    public LintResult Run()
    {
        var files = Directory.EnumerateFiles(options.AppDir, "*.xaml", SearchOption.AllDirectories)
            .Where(p => !SourceIndex.IsBuildOutput(p, options.AppDir)).Order(StringComparer.Ordinal).ToList();
        foreach (var f in files) LintFile(f);
        foreach (var dup in _result.Classes.GroupBy(c => c.FullName).Where(g => g.Count() > 1))
            foreach (var c in dup.Skip(1))
                Error(c.File, 1, $"x:Class '{c.FullName}' is also declared by {Rel(dup.First().File)}");
        _result.FileCount = files.Count;
        return _result;
    }

    void LintFile(string path)
    {
        _file = path;
        _class = null;
        _ignored = [Blend];
        _reportedNamespaces = [];
        _names = [];
        _codeBehind = [];
        _handlerScope = [];

        XDocument doc;
        try { doc = XDocument.Load(path, LoadOptions.SetLineInfo); }
        catch (XmlException e) { Error(path, e.LineNumber, $"not well-formed XML: {e.Message}"); return; }
        var root = doc.Root;
        if (root is null) { Error(path, 1, "empty document"); return; }

        // mc:Ignorable prefixes are design-time only; the compiler never sees them.
        foreach (var prefix in (root.Attribute(Mc + "Ignorable")?.Value ?? "").Split(' ', StringSplitOptions.RemoveEmptyEntries))
            if (root.GetNamespaceOfPrefix(prefix) is { } ns) _ignored.Add(ns);

        if (root.Attribute(X + "Class")?.Value is { } xClass)
        {
            if (!QualifiedIdentifier.IsMatch(xClass)) Error(path, Line(root), $"x:Class '{xClass}' is not a valid CLR type name");
            _class = new XamlClass { FullName = xClass, File = path };
            _result.Classes.Add(_class);
            CheckCodeBehind(root);
        }
        Visit(root, isRoot: true, inTemplate: false);
    }

    void CheckCodeBehind(XElement root)
    {
        var cls = _class!;
        var partials = sources.PartialsOf(cls.Name);
        var matching = partials.Where(f => SourceIndex.DeclaresNamespace(f, cls.Namespace)).ToList();
        if (partials.Count == 0)
            Error(_file, Line(root), $"no 'partial class {cls.Name}' in any .cs under {Rel(options.AppDir)} for x:Class '{cls.FullName}'");
        else if (matching.Count == 0)
            Error(_file, Line(root), $"'partial class {cls.Name}' exists ({Rel(partials[0].Path)}) but not in namespace '{cls.Namespace}'");
        _codeBehind = matching.Count > 0 ? matching : partials;

        foreach (var f in _codeBehind)
            if (Regex.IsMatch(f.Code, @"\bvoid\s+InitializeComponent\s*\("))
                Error(f.Path, SourceIndex.LineOf(f.Code, "InitializeComponent"), $"{cls.Name} declares InitializeComponent(); the XAML compiler generates it, remove the declaration");
        cls.WantsConnect = SourceIndex.Mentions(_codeBehind, @"\bConnect\s*\(");
        // Handlers may be inherited from an app-declared base class (the compiler wires `this.Handler`).
        _handlerScope = sources.WithAncestors(_codeBehind, cls.Name);
    }

    void Visit(XElement e, bool isRoot, bool inTemplate)
    {
        var ns = e.Name.Namespace;
        if (_ignored.Contains(ns)) return;
        var local = e.Name.LocalName;
        var line = Line(e);

        if (local.Contains('.'))
        {
            // Property element <Owner.Property>: the owner is a type and must have the property (instance or attached).
            var dot = local.IndexOf('.');
            var owner = ResolveType(ns, local[..dot], e, line, $"in property element <{Prefix(e, ns)}{local}>");
            var prop = local[(dot + 1)..];
            if (owner?.Metadata is { } ot && !types.HasInstanceProperty(ot, prop) && !types.HasAttachedProperty(ot, prop))
                Error(_file, line, $"'{owner.FullName}' has no property '{prop}'");
            foreach (var child in e.Elements()) Visit(child, false, inTemplate);
            return;
        }

        if (ns == Presentation && IntrinsicElements.TryGetValue(local, out var intrinsicAttributes))
        {
            VisitIntrinsic(e, local, intrinsicAttributes, isRoot, inTemplate);
            return;
        }

        var type = ResolveType(ns, local, e, line);
        if (isRoot && _class is not null) _class.BaseTypeFullName = type?.FullName;
        // Names inside a DataTemplate / ControlTemplate / ItemsPanelTemplate are template-scoped: no fields.
        var childInTemplate = inTemplate || (type?.Metadata is { } mt && types.DerivesFrom(mt, FrameworkTemplate));

        foreach (var a in e.Attributes())
        {
            if (a.IsNamespaceDeclaration) continue;
            var ans = a.Name.Namespace;
            if (ans == Mc || ans == Xml || _ignored.Contains(ans)) continue;
            var aline = Line(a);
            var aname = a.Name.LocalName;

            if (XBind.IsMatch(a.Value))
                Error(_file, aline, $"{aname}=\"{a.Value}\": x:Bind is not supported by xamlcheck (its code comes from the XAML compiler); use {{Binding}} or code-behind");

            if (ans == X) { VisitXAttribute(a, e, isRoot, inTemplate, type, aline); continue; }

            if (aname.Contains('.'))
            {
                // Attached property Owner.Property="…"; an unprefixed attribute lives in its element's namespace.
                var dot = aname.IndexOf('.');
                var owner = ResolveType(ans == XNamespace.None ? ns : ans, aname[..dot], e, aline, $"in attached property {(ans == XNamespace.None ? "" : Prefix(e, ans))}{aname}=\"…\"");
                var prop = aname[(dot + 1)..];
                if (owner?.Metadata is { } ot && !types.HasAttachedProperty(ot, prop) && !types.HasInstanceProperty(ot, prop))
                    Error(_file, aline, $"'{owner.FullName}' has no attached property '{prop}'");
                continue;
            }
            if (ans != XNamespace.None) { Error(_file, aline, $"unexpected prefixed attribute '{a.Name}'"); continue; }
            if (type?.Metadata is not { } t) continue; // app-declared type, or already reported as unresolved

            if (types.HasInstanceProperty(t, aname))
            {
                // FrameworkElement.Name is x:Name by another spelling; the compiler emits a field for it too.
                if (aname == "Name" && types.DerivesFrom(t, FrameworkElement))
                    AddField(a.Value, type, isRoot, inTemplate, aline, e.Attribute(X + "FieldModifier")?.Value);
                continue;
            }
            if (types.HasEvent(t, aname)) { CheckHandler(aname, a.Value, aline); continue; }
            Error(_file, aline, $"'{type.FullName}' has no property or event '{aname}'");
        }

        foreach (var child in e.Elements()) Visit(child, false, childInTemplate);
    }

    void VisitIntrinsic(XElement e, string name, HashSet<string> allowed, bool isRoot, bool inTemplate)
    {
        foreach (var a in e.Attributes())
        {
            if (a.IsNamespaceDeclaration) continue;
            var ans = a.Name.Namespace;
            if (ans == Mc || ans == Xml || _ignored.Contains(ans)) continue;
            var aline = Line(a);
            if (ans == X) { VisitXAttribute(a, e, isRoot, inTemplate, null, aline); continue; }
            if (ans != XNamespace.None || !allowed.Contains(a.Name.LocalName))
                Error(_file, aline, $"<{name}> has no property '{a.Name.LocalName}' (it takes {string.Join(", ", allowed)})");
        }
        if (!allowed.Any(p => e.Attribute(p) is not null))
            Error(_file, Line(e), $"<{name}> needs {string.Join(" or ", allowed)}");
        foreach (var child in e.Elements()) Error(_file, Line(child), $"<{name}> takes no content");
    }

    void VisitXAttribute(XAttribute a, XElement e, bool isRoot, bool inTemplate, XamlType? type, int line)
    {
        switch (a.Name.LocalName)
        {
            case "Class":
                if (!isRoot) Error(_file, line, "x:Class is only allowed on the root element");
                break;
            case "Name":
                AddField(a.Value, type, isRoot, inTemplate, line, e.Attribute(X + "FieldModifier")?.Value);
                break;
            default:
                if (!XAttributes.Contains(a.Name.LocalName)) Warn(_file, line, $"unknown attribute x:{a.Name.LocalName}");
                break;
        }
    }

    void AddField(string name, XamlType? type, bool isRoot, bool inTemplate, int line, string? modifier)
    {
        if (!Identifier.IsMatch(name)) { Error(_file, line, $"x:Name '{name}' is not a valid C# identifier"); return; }
        if (_class is null) { Warn(_file, line, $"x:Name '{name}' in a file without x:Class: no field is generated for it"); return; }
        if (isRoot) { log.Info($"{Rel(_file)}:{line}: x:Name '{name}' on the root is 'this'; no field"); return; }
        if (inTemplate) return;
        if (!_names.Add(name)) { Error(_file, line, $"x:Name '{name}' is used twice in this file"); return; }
        _class.Fields.Add(new XamlField(name, type?.FullName ?? "System.Object", modifier ?? "internal", line));
    }

    void CheckHandler(string eventName, string handler, int line)
    {
        if (handler.StartsWith('{')) return; // a markup extension, not a handler; x:Bind was reported above
        if (!Identifier.IsMatch(handler)) { Error(_file, line, $"{eventName}=\"{handler}\" is not a method name"); return; }
        if (_class is null) { Error(_file, line, $"{eventName}=\"{handler}\": an event handler needs a code-behind (x:Class)"); return; }
        if (!SourceIndex.HasMethod(_handlerScope, handler))
            Error(_file, line, $"{eventName}=\"{handler}\": no method '{handler}(' declared in the code-behind of {_class.FullName} ({string.Join(", ", _handlerScope.Select(f => Rel(f.Path)))})");
    }

    /// <param name="role">Null for an element; otherwise where the type name was used, for the message ("in attached property Gridd.Row=…").</param>
    XamlType? ResolveType(XNamespace ns, string name, XElement at, int line, string? role = null)
    {
        string What(string prefixed) => role is null ? $"unknown element <{prefixed}>" : $"unknown type '{prefixed}' {role}";
        if (ns == X)
        {
            if (XTypes.TryGetValue(name, out var sys)) return new XamlType(sys, types.Find(sys));
            Error(_file, line, What($"x:{name}"));
            return null;
        }
        if (ns == Presentation)
        {
            var t = types.FindInDefaultNamespace(name, out var ambiguous);
            if (t is null)
            {
                Error(_file, line, $"{What(name)}: no public type '{name}' in the Microsoft.UI.Xaml namespaces of the referenced WinUI assemblies");
                return null;
            }
            if (ambiguous) log.Info($"{Rel(_file)}:{line}: '{name}' exists in several default-namespace namespaces; using {t.FullName}");
            return new XamlType(t.FullName!, t);
        }
        if (ns.NamespaceName.StartsWith(Using, StringComparison.Ordinal))
        {
            var clrNs = ns.NamespaceName[Using.Length..];
            var full = clrNs.Length == 0 ? name : $"{clrNs}.{name}";
            if (types.Find(full) is { } t) return new XamlType(full, t);
            switch (sources.FindClass(clrNs, name))
            {
                case SourceIndex.ClassMatch.Exact:
                    return new XamlType(full, null);
                case SourceIndex.ClassMatch.NamespaceDiffers:
                    Warn(_file, line, $"'{name}' is declared in the sources but not in namespace '{clrNs}'");
                    return new XamlType(full, null);
                default:
                    Error(_file, line, $"{What($"{Prefix(at, ns)}{name}")}: no type '{full}' in the referenced assemblies or in the app's sources");
                    return null;
            }
        }
        if (_reportedNamespaces.Add(ns.NamespaceName))
            Error(_file, line, $"unsupported xmlns '{ns}': WinUI 3 XAML maps CLR namespaces with xmlns:p=\"using:Namespace\"");
        return null;
    }

    static string Prefix(XElement at, XNamespace ns) => at.GetPrefixOfNamespace(ns) is { Length: > 0 } p ? p + ":" : "";
    static int Line(IXmlLineInfo li) => li.HasLineInfo() ? li.LineNumber : 0;
    string Rel(string path) => Path.GetRelativePath(options.RepoRoot, path);
    void Error(string file, int line, string message) => _result.Diagnostics.Add(new Diagnostic(true, file, line, message));
    void Warn(string file, int line, string message) => _result.Diagnostics.Add(new Diagnostic(false, file, line, message));
}
