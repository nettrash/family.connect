# xamlcheck

The compile check for `src/FamilyConnect.App` that runs where the WinUI XAML compiler does not: macOS and
Linux. FamilyConnect.App is a WinUI 3 app, and the XAML compiler (`XamlCompiler.exe`, net472) is a Windows
binary, so `dotnet build src/FamilyConnect.App` fails off Windows before a single line of C# is compiled. This
tool gives the same feedback loop for everything short of that compiler.

```sh
tools/xamlcheck/run.sh                # this checkout; lint + shadow build, exit 1 on any error
tools/xamlcheck/run.sh -v             # also the dotnet invocations, references and every x:Name
tools/xamlcheck/run.sh --no-build     # lint and write the stubs only
dotnet run --project tools/xamlcheck -- <repo-root> [options]   # the same without the wrapper
```

## What it does

1. **Shadow project.** Writes `FamilyConnect.App.Shadow.csproj` outside the repo (`$XAMLCHECK_SHADOW_DIR`,
   else `<tmp>/xamlcheck-shadow`): a `net10.0-windows` class library with the app's
   TargetFramework, minimum version and package versions copied from `FamilyConnect.App.csproj`, the repo's
   `Directory.Build.props`, `EnableWindowsTargeting`, every `ProjectReference` the app csproj has
   (`FamilyConnect.Core`, `FamilyConnect.App.Logic`), and `src/FamilyConnect.App/**/*.cs` as its sources.
   Restores it and asks MSBuild for the resolved `ReferencePath` items, so the check runs against
   exactly the assemblies `csc` would see (Microsoft.WinUI.dll, Microsoft.Windows.SDK.NET.dll,
   WinRT.Runtime.dll, the WebView2 projection, the .NET reference pack).
2. **XAML lint.** Reads those assemblies as metadata (`System.Reflection.MetadataLoadContext`;
   nothing WinRT is ever executed) and checks every `*.xaml`: each element is a public type in the
   `Microsoft.UI.Xaml.*` namespaces (or, for `using:` prefixes, in the referenced assemblies or the
   app's own sources — plus the parser's own `<StaticResource x:Key="…" ResourceKey="…"/>` object
   element, which no type backs), each attribute is a property, event or attached property of that type,
   each event handler is a method *declared* in the code-behind (any partial of the class, or an
   app-declared base class — the compiler wires `this.Handler`), each `x:Class` has a
   `partial class` in a `.cs`, and no `{x:Bind}` is used. Reported as `file:line: error: message`.
3. **Stubs.** Writes `Xaml.Stubs.g.cs` into the shadow directory: for every `x:Class` the partial the
   XAML compiler would generate, reduced to one field per `x:Name` (typed as resolved, template-scoped
   names excluded) and an empty `InitializeComponent()`.
4. **Shadow build.** `dotnet build` of the shadow project. Errors are the real C# compiler's, with
   paths into `src/FamilyConnect.App`.

## What it proves and what it does not

Proves: the code-behind compiles against the real WinUI 3 / Windows App SDK API surface at the
pinned package versions, every XAML element, property, event and handler resolves, and the two
halves agree on the `x:Name` fields.

Does not prove: anything only the XAML compiler checks (attribute value conversion, markup-extension
syntax, `{x:Bind}` code generation, `StaticResource`/`ThemeResource` lookups, styles and templates
against their `TargetType`), the *signature* of an event handler (the name is matched textually:
comments and string literals are blanked first, and a call such as `=> Handler()` or
`return Handler()` does not count as a declaration, but parameter types and the return type are
not compared with the event's delegate), anything the linker or MSIX packaging checks, and runtime
behaviour. The real proof remains the `windows-latest` job in `.github/workflows/ci.yml`.

## Notes

- `x:Bind` is reported as an error on purpose: its code comes from the XAML compiler and cannot be
  stubbed, so the app avoids it (`{Binding}` or code-behind instead).
- `Name="…"` on a FrameworkElement is treated as `x:Name`, as the XAML compiler does. An `x:Name`
  on the root element gets no field (it is `this`).
- `Platform`/`RuntimeIdentifier` are set inside the shadow csproj, not passed as `-p:` globals: a
  global RID flows into the `FamilyConnect.Core` project reference and re-restores it for `win-x64`.
- MRT Core's PRI generation (`MakePri.exe`, a Windows binary) is switched off in the shadow with
  `EnableCoreMrtTooling=false`; it runs after the compile and a library needs no PRI.
- The tool itself is plain `net10.0` and part of `FamilyConnect.slnx`; it needs the Windows App SDK packages
  in the NuGet cache, which the shadow restore fetches.
- The shadow build writes only under the shadow directory — with one inherent exception: `FamilyConnect.Core`
  and `FamilyConnect.App.Logic` are real `ProjectReference`s, so an out-of-date one is built incrementally in
  place (`src/FamilyConnect.Core/obj`, `src/FamilyConnect.App.Logic/obj`, `bin`), exactly as `dotnet build src/FamilyConnect.App` or
  `dotnet test` would. Nothing under `src/FamilyConnect.App` or `tests/` is ever written.
