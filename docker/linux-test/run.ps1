<#
.SYNOPSIS
  Run rustshot's Linux live tests in Docker (X11 under Xvfb, or Wayland under
  headless sway with xdg-desktop-portal-wlr). See docker/linux-test/README.md.

.EXAMPLE
  .\docker\linux-test\run.ps1 -Mode x11
  .\docker\linux-test\run.ps1 -Mode wayland -Out D:\rs-out -Rebuild
#>
param(
    [ValidateSet('x11', 'wayland', 'shell')]
    [string]$Mode = 'x11',
    # Host folder for screenshots, recordings and logs (mounted at /out).
    [string]$Out = (Join-Path $env:USERPROFILE 'rustshot-docker-out'),
    [string]$Image = 'rustshot-linux-test',
    # Git revision to test (default HEAD, i.e. committed code); 'worktree'
    # copies the working tree with its uncommitted edits.
    [string]$Ref = 'HEAD',
    # Rebuild the image even when it exists.
    [switch]$Rebuild
)
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$repo = (Resolve-Path (Join-Path $here '..\..')).Path

docker image inspect $Image *> $null
if ($Rebuild -or $LASTEXITCODE -ne 0) {
    docker build -t $Image $here
    if ($LASTEXITCODE -ne 0) { throw "docker build failed" }
}
New-Item -ItemType Directory -Force $Out | Out-Null

# The repo is read-only at /src (copied to /work inside); the build output
# and the cargo registry live in named volumes, i.e. on Docker's disk.
$run = @(
    'run', '--rm', '--init',
    '--shm-size=1g',
    '-v', "${repo}:/src:ro",
    '-v', "${Out}:/out",
    '-v', 'rustshot-target:/target',
    '-v', 'rustshot-cargo:/usr/local/cargo/registry',
    '-e', 'RS_TEST_TIMEOUT', '-e', 'RS_ONLY', '-e', 'RS_GST_CHECK', '-e', 'RS_WL_ANIM', '-e', "RS_REF=$Ref"
)
if ($Mode -eq 'shell') {
    docker @run -it --entrypoint bash $Image
} else {
    docker @run $Image $Mode
}
$rc = $LASTEXITCODE
Write-Host "outputs: $(Join-Path $Out $Mode)"
exit $rc
