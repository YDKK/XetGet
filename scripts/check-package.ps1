#Requires -Version 7.3
# Requires PowerShell 7 (pwsh). All disposable installation files stay in OS TEMP.
param([switch]$AllowDirty)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

$metadata = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
$package = $metadata.packages | Where-Object name -eq 'xetget'
$arguments = @('package', '--locked')
if ($AllowDirty) { $arguments += '--allow-dirty' }
& cargo @arguments

$work = Join-Path ([IO.Path]::GetTempPath()) ('xetget-package-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $work | Out-Null
try {
    $archive = Join-Path $metadata.target_directory "package/xetget-$($package.version).crate"
    tar -xzf $archive -C $work
    $source = Join-Path $work "xetget-$($package.version)"
    $install = Join-Path $work 'install'
    $build = Join-Path $work 'target'
    # Exercise the packaged tests too: they need the bundled source and patches.
    cargo test --locked --manifest-path "$source/Cargo.toml" --target-dir $build --all-targets
    cargo install --locked --path $source --root $install --target-dir $build
    # Also cover the default cargo install dependency resolution used by users.
    cargo install --force --path $source --root $install --target-dir $build
    $binary = Join-Path $install ('bin/xetget' + $(if ($IsWindows) { '.exe' } else { '' }))
    $version = & $binary --version
    if ($version -ne "xetget $($package.version)") { throw "Unexpected version: $version" }
    & $binary --help
} finally {
    # $work is a uniquely created, absolute child of OS TEMP, never a caller path.
    $resolved = (Resolve-Path -LiteralPath $work).Path
    $tempRoot = (Resolve-Path -LiteralPath ([IO.Path]::GetTempPath())).Path.TrimEnd([IO.Path]::DirectorySeparatorChar)
    if ((Split-Path -Parent $resolved) -ne $tempRoot) { throw "Refusing cleanup outside OS TEMP: $resolved" }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
