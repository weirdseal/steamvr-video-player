[CmdletBinding()]
param(
    [switch]$Gnu,
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
if (!$SkipBuild) {
    & (Join-Path $PSScriptRoot 'build.ps1') -Gnu:$Gnu
}

$project = Split-Path -Parent $PSScriptRoot
$target = if ($Gnu) { 'x86_64-pc-windows-gnu' } else { 'x86_64-pc-windows-msvc' }
$output = Join-Path $project "target\$target\release"
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem

$files = @(
    'steamvr-video-player.exe',
    'libmpv-2.dll',
    'openxr_loader.dll',
    'LICENSE',
    'LICENSE-libmpv.txt',
    'LICENSE-OpenXR.txt',
    'OFL-NotoSansKR.txt'
)
foreach ($file in $files) {
    $source = Join-Path $output $file
    if (!(Test-Path -LiteralPath $source -PathType Leaf)) { throw "Missing release file: $source" }
}

$destination = Join-Path $output 'steamvr-video-player-windows-x64.zip'
$temporary = "$destination.$([Guid]::NewGuid().ToString('N')).tmp"
try {
    $archive = [IO.Compression.ZipFile]::Open($temporary, [IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($file in $files) {
            [void][IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
                $archive,
                (Join-Path $output $file),
                "steamvr-video-player/$file",
                [IO.Compression.CompressionLevel]::Optimal
            )
        }
    } finally {
        $archive.Dispose()
    }

    if ([IO.File]::Exists($destination)) {
        [IO.File]::Replace($temporary, $destination, [NullString]::Value)
    } else {
        [IO.File]::Move($temporary, $destination)
    }
    Write-Host ('{0} ({1:N2} MiB)' -f $destination, ((Get-Item -LiteralPath $destination).Length / 1MB))
} finally {
    if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary }
}
