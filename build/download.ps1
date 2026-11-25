function Get-VerifiedDownload($Directory, $Name, $Url, $Sha256) {
    $destination = Join-Path $Directory $Name
    if ((Test-Path -LiteralPath $destination -PathType Leaf) -and
        (Get-FileHash -LiteralPath $destination).Hash -eq $Sha256) {
        return $destination
    }

    New-Item -ItemType Directory -Path $Directory -Force | Out-Null
    $download = Join-Path $Directory ("{0}.{1}.download" -f $Name, [Guid]::NewGuid())
    try {
        Write-Host "Downloading $Name"
        Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $download
        if ((Get-FileHash -LiteralPath $download).Hash -ne $Sha256) {
            throw "SHA-256 verification failed for $Name."
        }
        Move-Item -LiteralPath $download -Destination $destination -Force
    } finally {
        if (Test-Path -LiteralPath $download) { Remove-Item -LiteralPath $download }
    }
    return $destination
}
