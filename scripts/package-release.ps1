#Requires -Version 7.3
# Requires PowerShell 7. Build and package a native release for cargo-binstall.
param([Parameter(Mandatory)][string]$Target)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

$metadata = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
$package = $metadata.packages | Where-Object name -eq 'xetget'
$cargoHost = ((rustc -vV | Select-String '^host: ').Line -replace '^host: ', '')
if ($cargoHost -ne $Target) { throw "Use a native runner for $Target (host: $cargoHost)" }

cargo build --locked --release --target $Target
$binaryName = 'xetget' + $(if ($Target -like '*-windows-*') { '.exe' } else { '' })
$binary = Join-Path $metadata.target_directory "$Target/release/$binaryName"
$version = & $binary --version
if ($version -ne "xetget $($package.version)") { throw "Unexpected version: $version" }
& $binary --help

$dist = Join-Path (Get-Location) 'dist'
New-Item -ItemType Directory -Force -Path $dist | Out-Null
$work = Join-Path ([IO.Path]::GetTempPath()) ('xetget-release-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $work | Out-Null
try {
    Copy-Item -LiteralPath $binary -Destination (Join-Path $work $binaryName)
    Copy-Item -LiteralPath 'README.md', 'LICENSE' -Destination $work
    Copy-Item -LiteralPath 'vendor/hf-hub/LICENSE' -Destination (Join-Path $work 'LICENSE-hf-hub')
    Copy-Item -LiteralPath 'vendor/hf-hub/XETGET-PATCH.md' -Destination $work
    $extension = if ($Target -like '*-windows-*') { 'zip' } else { 'tgz' }
    $archiveName = "xetget-$($package.version)-$Target.$extension"
    $archive = Join-Path $dist $archiveName
    if (Test-Path -LiteralPath $archive) { throw "Archive already exists: $archive" }
    if ($extension -eq 'zip') {
        Compress-Archive -Path "$work/*" -DestinationPath $archive
    } else {
        tar -czf $archive -C $work .
    }
    $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText("$archive.sha256", "$hash  $archiveName`n", [Text.UTF8Encoding]::new($false))
    if ($env:GITHUB_OUTPUT) { "archive=$archive" | Add-Content -LiteralPath $env:GITHUB_OUTPUT }
    Write-Output "Created $archive"
} finally {
    # $work is a uniquely created, absolute child of OS TEMP, never a caller path.
    $resolved = (Resolve-Path -LiteralPath $work).Path
    $tempRoot = (Resolve-Path -LiteralPath ([IO.Path]::GetTempPath())).Path.TrimEnd([IO.Path]::DirectorySeparatorChar)
    if ((Split-Path -Parent $resolved) -ne $tempRoot) { throw "Refusing cleanup outside OS TEMP: $resolved" }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
