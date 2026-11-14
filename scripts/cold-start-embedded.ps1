# Runs embedded-migrate from a temporary directory while the auth and job
# migration JSON directories are renamed out of the source tree.
# The process must not open CARGO_MANIFEST_DIR. A PostgreSQL URL is accepted
# only when its database name contains fexu_test. Without -DatabaseUrl the
# script creates a fresh SQLite file and does not read the ambient DATABASE_URL.
param(
    [string]$DatabaseUrl
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$built = Join-Path $root "target\debug\embedded-migrate.exe"
if (-not (Test-Path $built)) {
    cargo build -p fexu-cli --bin embedded-migrate
}
if (-not (Test-Path $built)) {
    throw "embedded-migrate.exe was not built"
}

$dest = Join-Path $env:TEMP ("fexu-cold-" + [guid]::NewGuid().ToString("N"))
$work = Join-Path $env:TEMP ("fexu-cold-work-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $dest, $work | Out-Null
$bin = Join-Path $dest "embedded-migrate.exe"
Copy-Item $built $bin
if ($bin.StartsWith($root, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "binary was not moved out of the source tree"
}

if ([string]::IsNullOrWhiteSpace($DatabaseUrl)) {
    $sqlite = (Join-Path $work "app.sqlite").Replace("\", "/")
    $DatabaseUrl = "sqlite://${sqlite}?mode=rwc"
} elseif ($DatabaseUrl -like "postgres*" -and $DatabaseUrl -notlike "*fexu_test*") {
    throw "refusing a PostgreSQL URL whose database name does not contain fexu_test"
}

$auth = Join-Path $root "crates\fexu-auth\migrations"
$jobs = Join-Path $root "crates\fexu-jobs\migrations"
$authHide = Join-Path $root "crates\fexu-auth\migrations.cold-hidden"
$jobsHide = Join-Path $root "crates\fexu-jobs\migrations.cold-hidden"
if ((Test-Path $authHide) -or (Test-Path $jobsHide)) {
    throw "a previous cold start left migrations.cold-hidden in place"
}

try {
    Move-Item $auth $authHide
    Move-Item $jobs $jobsHide
    if ((Test-Path $auth) -or (Test-Path $jobs)) {
        throw "migration source directories are still visible"
    }
    Write-Host "cold-start source_migrations=hidden binary=$bin"
    $previous = $env:DATABASE_URL
    $env:DATABASE_URL = $DatabaseUrl
    try {
        & $bin
        if ($LASTEXITCODE -ne 0) {
            throw "embedded-migrate exited $LASTEXITCODE"
        }
    } finally {
        if ($null -eq $previous) {
            Remove-Item Env:DATABASE_URL -ErrorAction SilentlyContinue
        } else {
            $env:DATABASE_URL = $previous
        }
    }
} finally {
    if ((Test-Path $authHide) -and -not (Test-Path $auth)) {
        Move-Item $authHide $auth
    }
    if ((Test-Path $jobsHide) -and -not (Test-Path $jobs)) {
        Move-Item $jobsHide $jobs
    }
}
