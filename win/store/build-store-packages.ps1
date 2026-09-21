# Build the Microsoft Store upload packages: one .msixupload per architecture, pointed at the default server.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File win/store/build-store-packages.ps1 [-OutDir <folder>] [-Server <url>]
#
# Upload BOTH files to the same Partner Center submission (Packages step): the Store serves each PC the one for its
# architecture, so there is nothing to bundle. They are UNSIGNED on purpose - the Store signs what it publishes.
#
# What each setting is for:
#   UapAppxPackageBuildMode=StoreUpload   writes the .msixupload (the package plus its symbols) Partner Center takes
#   AppxBundle=Never                      one package per architecture rather than a bundle
#   AppxPackageSigningEnabled=false       no certificate: the Store signs
#   EnableWinAppRunSupport=false          no dev identity registration, which only `dotnet run` wants
#   FamilyConnectDefaultServer            the server a first launch opens on (Services/DefaultServer.cs)
#
# Visual Studio's MSBuild with the "Desktop development with C++" workload, as the CI win-app job uses.
#
# THE VERSION IS STAMPED, NOT HAND-WRITTEN. Major.Minor comes from win/Directory.Build.props - the one place the
# marketing version lives - and the build number from `git rev-list --count HEAD`, or -Build if you name one. The Store
# REFUSES A VERSION IT HAS ALREADY SEEN, and a hand-written 1.1.0.0 is exactly the second submission that gets refused;
# a commit count only ever rises. Package.appxmanifest is put back as it was when the build finishes, so this leaves no
# edit behind. (CI stamps the same field from its run number, for a package that is only ever tested - the two numbers
# are separate streams, and only the ones from this script reach the Store.)
param(
    [string]$OutDir = '',
    [string]$Server = 'https://fc.nettrash.me',
    [int]$Build = 0
)
$ErrorActionPreference = 'Stop'

$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
if (-not $OutDir) { $OutDir = Join-Path $repo ('win\AppPackages\store-' + (Get-Date -Format 'yyyyMMdd-HHmmss')) }
New-Item -ItemType Directory -Force $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path.TrimEnd('\') + '\'

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$msbuild = & $vswhere -latest -prerelease -requires Microsoft.Component.MSBuild -find 'MSBuild\**\Bin\MSBuild.exe' | Select-Object -First 1
if (-not $msbuild) { throw 'Visual Studio MSBuild was not found - install Visual Studio with the Desktop development with C++ workload' }

# The marketing version: selected, counted, and only then read. `.Project.PropertyGroup.Version` silently answers
# nothing once a second PropertyGroup exists, which would stamp "..0.0" and carry on.
$props = Join-Path $repo 'win\Directory.Build.props'
$found = @([xml](Get-Content $props) | ForEach-Object { $_.SelectNodes('//PropertyGroup/Version') } |
    ForEach-Object { $_.InnerText.Trim() } | Where-Object { $_ })
if ($found.Count -ne 1) { throw "expected one <Version> in $props, found $($found.Count)" }
if ($found[0] -notmatch '^(\d+)\.(\d+)') { throw "no Major.Minor in $props : '$($found[0])'" }
$major, $minor = $Matches[1], $Matches[2]
if ($Build -le 0) {
    $Build = [int](& git -C $repo rev-list --count HEAD)
    if ($LASTEXITCODE -ne 0 -or $Build -le 0) { throw 'the build number could not be counted from git - pass -Build' }
}
if ($Build -gt 65535) { throw "build number $Build is past the 16-bit MSIX limit" }
$packageVersion = "$major.$minor.$Build.0"

# A targeted rewrite of Identity/@Version, not an XML round-trip: the manifest carries comments that say why its
# identity is what it is, and reformatting them is not this script's business. `[^>]` keeps the match inside the single
# <Identity ...> tag and `\bVersion` keeps it off the tail of MinVersion - a looser pattern rewrote TargetDeviceFamily's
# MinVersion when the Identity's own attribute was missing, which is a green build claiming to install on Windows 1.1.
$manifest = Join-Path $repo 'win\src\FamilyConnect.App\Package.appxmanifest'
$pattern = '(<Identity\b[^>]*?\bVersion=")[^"]+(")'
$original = Get-Content $manifest -Raw
# Asked before writing, and checked after: "nothing changed" is not the test for a missing attribute, because a re-run
# stamps the value already there.
if ($original -notmatch $pattern) { throw "Identity/@Version was not found in $manifest" }
Set-Content $manifest -NoNewline -Value ([regex]::Replace($original, $pattern, "`${1}$packageVersion`${2}", 1))
if ((Get-Content $manifest -Raw) -notmatch [regex]::Escape("Version=`"$packageVersion`"")) {
    throw "$manifest does not carry $packageVersion after patching"
}
Write-Host "=== version $packageVersion"

$project = Join-Path $repo 'win\src\FamilyConnect.App\FamilyConnect.App.csproj'
try {
foreach ($platform in 'x64', 'ARM64') {
    $rid = if ($platform -eq 'ARM64') { 'win-arm64' } else { 'win-x64' }
    Write-Host "=== $platform"
    & $msbuild $project -restore -nologo -v:minimal `
        -p:Configuration=Release -p:Platform=$platform -p:RuntimeIdentifier=$rid `
        -p:GenerateAppxPackageOnBuild=true -p:UapAppxPackageBuildMode=StoreUpload -p:AppxBundle=Never `
        -p:AppxPackageSigningEnabled=false -p:EnableWinAppRunSupport=false `
        -p:AppxPackageDir=$OutDir -p:FamilyConnectDefaultServer=$Server -p:Version="$major.$minor.$Build"
    if ($LASTEXITCODE -ne 0) { throw "the $platform package did not build" }
}
}
finally {
    # The stamp belongs to the package, not to the working tree - put the manifest back however this ended.
    Set-Content $manifest -NoNewline -Value $original
}

$uploads = @(Get-ChildItem $OutDir -Filter *.msixupload)
if ($uploads.Count -ne 2) { throw "expected two .msixupload files in $OutDir, found $($uploads.Count)" }
Write-Host ''
Write-Host "Upload these two to Partner Center as version $packageVersion (the _Test folders beside them are for sideloading, not the Store):"
$uploads | ForEach-Object { Write-Host ('  {0}  ({1:N0} MB)' -f $_.FullName, ($_.Length / 1MB)) }
