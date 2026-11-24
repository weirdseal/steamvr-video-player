[CmdletBinding()]
param(
    [switch]$DebugBuild,
    [switch]$Gnu,
    [string]$BuildDirectory = ''
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$project = Split-Path -Parent $PSScriptRoot
if (!$BuildDirectory) { $BuildDirectory = Join-Path $project 'target' }
$BuildDirectory = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($BuildDirectory)
$vendorDirectory = Join-Path $BuildDirectory 'dependencies'
New-Item -ItemType Directory -Path $vendorDirectory -Force | Out-Null
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Get-VerifiedArchive($Name, $Url, $Sha256) {
    $archive = Join-Path $vendorDirectory $Name
    if ((Test-Path -LiteralPath $archive -PathType Leaf) -and (Get-FileHash -LiteralPath $archive).Hash -eq $Sha256) {
        return $archive
    }

    $download = Join-Path $vendorDirectory ("{0}.{1}.download" -f $Name, [Guid]::NewGuid())
    try {
        Write-Host "Downloading $Name"
        Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $download
        if ((Get-FileHash -LiteralPath $download).Hash -ne $Sha256) {
            throw "SHA-256 verification failed for $Name."
        }
        Move-Item -LiteralPath $download -Destination $archive -Force
    } finally {
        if (Test-Path -LiteralPath $download) { Remove-Item -LiteralPath $download }
    }
    return $archive
}

function Copy-ArchiveEntry($Archive, $Name, $Destination) {
    $zip = [System.IO.Compression.ZipFile]::OpenRead($Archive)
    try {
        $entry = $zip.GetEntry($Name)
        if (!$entry) { throw "$Name was missing from $Archive." }
        [System.IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $Destination, $true)
    } finally {
        $zip.Dispose()
    }
}

$mpvName = 'libmpv-v0.42.0-x86_64-w64-mingw32-lgpl.zip'
$mpv = Get-VerifiedArchive $mpvName `
    "https://github.com/mpv-player/mpv/releases/download/v0.42.0/$mpvName" `
    '8e5f77430efc53ef175804ba6618527e3af0448a0dcabf04d3cc72659d1a6fce'
$xrName = 'OpenXR.Loader.1.1.65.nupkg'
$xr = Get-VerifiedArchive $xrName `
    "https://github.com/KhronosGroup/OpenXR-SDK/releases/download/release-1.1.65/$xrName" `
    '05638c526f07ba3a5f1084bbc3308d654d5167f844ffa1e35515325cb72e0c34'

$target = 'x86_64-pc-windows-msvc'
$toolchain = @()
if ($Gnu) {
    $target = 'x86_64-pc-windows-gnu'
    $toolchain = @('+stable-x86_64-pc-windows-gnu')
}
$profile = if ($DebugBuild) { 'debug' } else { 'release' }
$cargoArgs = @('build', '--locked', '--target', $target, '--target-dir', $BuildDirectory)
if (!$DebugBuild) { $cargoArgs += '--release' }

Push-Location -LiteralPath $project
try {
    & cargo @toolchain @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw "Cargo build failed (exit code $LASTEXITCODE)." }
} finally {
    Pop-Location
}

$output = Join-Path (Join-Path $BuildDirectory $target) $profile
Copy-ArchiveEntry $mpv 'libmpv-2.dll' (Join-Path $output 'libmpv-2.dll')
Copy-ArchiveEntry $mpv 'LICENSE.LGPL' (Join-Path $output 'LICENSE-libmpv.txt')
Copy-ArchiveEntry $xr 'native/x64/release/bin/openxr_loader.dll' (Join-Path $output 'openxr_loader.dll')
Copy-Item -LiteralPath (Join-Path $project 'assets\OFL-NotoSansKR.txt') -Destination $output
Copy-Item -LiteralPath (Join-Path $project 'LICENSE') -Destination $output
Write-Host (Join-Path $output 'steamvr-video-player.exe')
