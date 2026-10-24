#!/bin/sh
# Regenerates the .NET fixtures with the installed SDK (offline-friendly: restore is separate).
# Usage: sh tests/fixtures/make_fixtures.sh
set -e
cd "$(dirname "$0")"
export DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1
rm -rf Shop TenProjects
# Shop: console app, class library, ASP.NET Core API and xUnit / NUnit / MSTest test projects.
mkdir Shop && cd Shop
dotnet new sln -n Shop --format sln
dotnet new classlib -n Shop.Core -o src/Shop.Core -f net8.0
dotnet new console -n Shop.Cli -o src/Shop.Cli -f net8.0
dotnet new webapi -n Shop.Api -o src/Shop.Api -f net8.0 --no-https --use-controllers
dotnet new xunit -n Shop.Core.XunitTests -o tests/Shop.Core.XunitTests -f net8.0
dotnet new nunit -n Shop.Core.NunitTests -o tests/Shop.Core.NunitTests -f net8.0
dotnet new mstest -n Shop.Core.MSTests -o tests/Shop.Core.MSTests -f net8.0
for p in src/Shop.Cli src/Shop.Api tests/Shop.Core.XunitTests tests/Shop.Core.NunitTests tests/Shop.Core.MSTests; do
  dotnet add $p reference src/Shop.Core/Shop.Core.csproj
done
dotnet sln add --solution-folder src src/Shop.Core src/Shop.Cli src/Shop.Api
dotnet sln add --solution-folder tests tests/Shop.Core.XunitTests tests/Shop.Core.NunitTests tests/Shop.Core.MSTests
cd ..
# TenProjects: ten class libraries in a .slnx, for the explorer and session tests.
mkdir TenProjects && cd TenProjects
dotnet new sln -n TenProjects --format slnx
for i in 01 02 03 04 05 06 07 08 09 10; do
  dotnet new classlib -n Module$i -o src/Module$i -f net8.0
  dotnet sln add --solution-folder src src/Module$i
done
