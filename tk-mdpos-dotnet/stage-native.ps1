<#
.SYNOPSIS
    Builds the tk-mdpos C ABI and stages the native binary where the .csproj expects it.

.DESCRIPTION
    The NuGet package carries native binaries under runtimes/{rid}/native/. Those are build
    outputs, not committed files, so something has to put them there before `dotnet pack`.
    In CI that is a native build on each runner; locally this script does the host RID.

    Only the RID matching the host is produced. On Windows that is win-x64; linux-x64 needs
    WSL, a container, or CI. Do not reach for a cross-linker on Windows before trying WSL.

.PARAMETER Configuration
    debug or release. Defaults to release, which is what ships.

.EXAMPLE
    ./stage-native.ps1
    dotnet pack TerraKernel.Mdpos -p:MdposAllowMissingRuntimes=true
#>
[CmdletBinding()]
param(
    [ValidateSet('debug', 'release')]
    [string]$Configuration = 'release'
)

$ErrorActionPreference = 'Stop'

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = Split-Path -Parent $here

# The cdylib is what ships: DllImport("tk_mdpos") resolves tk_mdpos.dll / libtk_mdpos.so.
# The staticlib is for iOS and for C callers who want one artifact, and is not packaged.
$rid, $artifact = if ($IsWindows -or $env:OS -eq 'Windows_NT') {
    'win-x64', 'tk_mdpos.dll'
} elseif ($IsLinux) {
    'linux-x64', 'libtk_mdpos.so'
} else {
    throw "Unsupported host for a NuGet RID. The package claims win-x64 and linux-x64 only; Apple platforms ship as an XCFramework via Swift Package Manager instead."
}

Write-Host "Building tk-mdpos-ffi ($Configuration) for $rid" -ForegroundColor Cyan

$cargoArgs = @('build', '-p', 'tk-mdpos-ffi')
if ($Configuration -eq 'release') { $cargoArgs += '--release' }

Push-Location $repo
try {
    # cargo writes its progress to stderr. Under Windows PowerShell 5.1 that is wrapped in
    # an ErrorRecord and, with ErrorActionPreference = Stop, aborts the script even though
    # the build succeeded. Exit code is the only honest signal from a native command.
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & cargo @cargoArgs
    }
    finally {
        $ErrorActionPreference = $previous
    }
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
}
finally {
    Pop-Location
}

$source = Join-Path $repo "target/$Configuration/$artifact"
if (-not (Test-Path -LiteralPath $source)) {
    throw "cargo reported success but $source is missing. Check the artifact name against target/$Configuration/ rather than assuming it."
}

$destDir = Join-Path $here "runtimes/$rid/native"
New-Item -ItemType Directory -Force -Path $destDir | Out-Null
Copy-Item -LiteralPath $source -Destination $destDir -Force

$staged = Join-Path $destDir $artifact
$size = (Get-Item -LiteralPath $staged).Length
Write-Host "Staged $rid -> $staged ($([math]::Round($size / 1KB)) KB)" -ForegroundColor Green

Write-Host ""
Write-Host "Note: only $rid was staged. A publishable package needs win-x64 and linux-x64," -ForegroundColor Yellow
Write-Host "and `dotnet pack` will refuse unless both are present." -ForegroundColor Yellow
