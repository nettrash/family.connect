#!/bin/sh
# xamlcheck — lint src/FamilyConnect.App's XAML and compile its code-behind against the real WinUI 3
# projection on macOS / Linux (see README.md).
#
#   tools/xamlcheck/run.sh [repo-root] [xamlcheck options]
#
# The repo root defaults to this checkout; everything else is passed through
# (`--no-build`, `-v`, `--shadow-dir <dir>`, `-c Release`, `--help`).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
if [ $# -gt 0 ] && [ "${1#-}" = "$1" ]; then root=$1; shift; fi
exec dotnet run --project "$here/xamlcheck.csproj" -- "$root" "$@"
