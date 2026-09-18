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
# Visual Studio's MSBuild with the "Desktop development with C++" workload, as the CI win-app job uses; the package
# version is the one written by hand in Package.appxmanifest, and the Store refuses a version it has already seen, so
# raise it before each new submission.
param(
    [string]$OutDir = '',
    [string]$Server = 'https://fc.nettrash.me'
)
$ErrorActionPreference = 'Stop'

$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
if (-not $OutDir) { $OutDir = Join-Path $repo ('win\AppPackages\store-' + (Get-Date -Format 'yyyyMMdd-HHmmss')) }
New-Item -ItemType Directory -Force $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path.TrimEnd('\') + '\'

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$msbuild = & $vswhere -latest -prerelease -requires Microsoft.Component.MSBuild -find 'MSBuild\**\Bin\MSBuild.exe' | Select-Object -First 1
if (-not $msbuild) { throw 'Visual Studio MSBuild was not found - install Visual Studio with the Desktop development with C++ workload' }

$project = Join-Path $repo 'win\src\FamilyConnect.App\FamilyConnect.App.csproj'
foreach ($platform in 'x64', 'ARM64') {
    $rid = if ($platform -eq 'ARM64') { 'win-arm64' } else { 'win-x64' }
    Write-Host "=== $platform"
    & $msbuild $project -restore -nologo -v:minimal `
        -p:Configuration=Release -p:Platform=$platform -p:RuntimeIdentifier=$rid `
        -p:GenerateAppxPackageOnBuild=true -p:UapAppxPackageBuildMode=StoreUpload -p:AppxBundle=Never `
        -p:AppxPackageSigningEnabled=false -p:EnableWinAppRunSupport=false `
        -p:AppxPackageDir=$OutDir -p:FamilyConnectDefaultServer=$Server
    if ($LASTEXITCODE -ne 0) { throw "the $platform package did not build" }
}

$uploads = @(Get-ChildItem $OutDir -Filter *.msixupload)
if ($uploads.Count -ne 2) { throw "expected two .msixupload files in $OutDir, found $($uploads.Count)" }
Write-Host ''
Write-Host 'Upload these two to Partner Center (the _Test folders beside them are for sideloading, not the Store):'
$uploads | ForEach-Object { Write-Host ('  {0}  ({1:N0} MB)' -f $_.FullName, ($_.Length / 1MB)) }
