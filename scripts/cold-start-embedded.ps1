# Runs embedded-migrate from a temporary directory while the auth and job
# migration JSON directories are renamed out of the source tree.
# The process must not open CARGO_MANIFEST_DIR. A PostgreSQL URL is accepted
# only when the decoded database name starts with fexu_test_, the host is the
# literal form 127.0.0.1, localhost, or [::1], and the URL has no query string.
# The host is not percent-decoded. The URL user must already be a
# disposable test role; this script does not verify that role's privileges. Without
# -DatabaseUrl the script creates a fresh SQLite file and does not read the
# ambient DATABASE_URL. Exit 0 also requires login=ok and a reported job.
# A killed process cannot restore the renamed directories; move
# migrations.cold-hidden back by hand.
param(
    [string]$DatabaseUrl
)

$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "cold-start-url.ps1")
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
} else {
    $reason = Get-ColdStartDatabaseRejection $DatabaseUrl
    if (-not [string]::IsNullOrWhiteSpace($reason)) {
        Write-Host $reason
        exit 2
    }
}

$auth = Join-Path $root "crates\fexu-auth\migrations"
$jobs = Join-Path $root "crates\fexu-jobs\migrations"
$authHide = Join-Path $root "crates\fexu-auth\migrations.cold-hidden"
$jobsHide = Join-Path $root "crates\fexu-jobs\migrations.cold-hidden"
if ((Test-Path $authHide) -or (Test-Path $jobsHide)) {
    throw "a previous cold start left migrations.cold-hidden in place"
}

function Restore-ColdStartDirectory {
    param([string]$Hidden, [string]$Source)
    if ((Test-Path -LiteralPath $Hidden) -and -not (Test-Path -LiteralPath $Source)) {
        Move-Item -LiteralPath $Hidden -Destination $Source -ErrorAction Stop
        return
    }
    if ((Test-Path -LiteralPath $Hidden) -and (Test-Path -LiteralPath $Source)) {
        throw "Both $Source and $Hidden exist. Leave both until you choose which copy to keep. A killed process does not run this cleanup."
    }
    if (-not (Test-Path -LiteralPath $Hidden) -and -not (Test-Path -LiteralPath $Source)) {
        throw "Neither $Source nor $Hidden exists. The migration directory was not restored. A killed process does not run this cleanup."
    }
}

$scriptError = $null
$restoreErrors = @()
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
        $log = Join-Path $work "embedded-migrate.log"
        & $bin > $log 2>&1
        $code = $LASTEXITCODE
        $output = ""
        if (Test-Path $log) {
            $output = Get-Content -Raw -Path $log
        }
        Write-Host $output
        if ($code -ne 0) {
            throw "embedded-migrate exited $code"
        }
        if ($output -notlike "*login=ok*") {
            throw "cold start did not log in"
        }
        if ($output -notlike "*job=*") {
            throw "cold start did not enqueue a job"
        }
    } finally {
        if ($null -eq $previous) {
            Remove-Item Env:DATABASE_URL -ErrorAction SilentlyContinue
        } else {
            $env:DATABASE_URL = $previous
        }
    }
} catch {
    $scriptError = $_
} finally {
    foreach ($pair in @(
            @{ Hidden = $authHide; Source = $auth },
            @{ Hidden = $jobsHide; Source = $jobs }
        )) {
        try {
            Restore-ColdStartDirectory -Hidden $pair.Hidden -Source $pair.Source
        } catch {
            $restoreErrors += $_.Exception.Message
            Write-Host $_.Exception.Message
        }
    }
}

if ($restoreErrors.Count -gt 0) {
    if ($null -ne $scriptError) {
        Write-Host "cold start failed before restore: $($scriptError.Exception.Message)"
    }
    Write-Host "cold start could not restore every migration directory. If migrations.cold-hidden remains, move it back before the next build."
    exit 1
}
if ($null -ne $scriptError) {
    Write-Host $scriptError.Exception.Message
    exit 1
}
Write-Host "cold-start login=ok job=enqueued"
