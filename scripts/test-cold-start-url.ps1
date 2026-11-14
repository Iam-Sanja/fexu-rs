# Calls the cold-start URL parser. It never opens a database connection.
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "cold-start-url.ps1")

$failed = 0

function Assert-Rejected {
    param([string]$Label, [string]$Url, [string]$Needle)
    $reason = Get-ColdStartDatabaseRejection $Url
    if ([string]::IsNullOrWhiteSpace($reason) -or $reason -notlike "*$Needle*") {
        Write-Host "FAIL $Label was not rejected ($reason)"
        $script:failed = 1
    } else {
        Write-Host "reject $Label"
    }
}

function Assert-Allowed {
    param([string]$Label, [string]$Url)
    $reason = Get-ColdStartDatabaseRejection $Url
    if (-not [string]::IsNullOrWhiteSpace($reason)) {
        Write-Host "FAIL $Label was rejected: $reason"
        $script:failed = 1
    } else {
        Write-Host "allow $Label"
    }
}

Assert-Rejected "user" "postgres://fexu_test@127.0.0.1:5432/fexu" "database name"
Assert-Rejected "password" "postgres://fexu:fexu_test@127.0.0.1:5432/production" "database name"
Assert-Rejected "query" "postgres://fexu@127.0.0.1:5432/fexu?application_name=fexu_test" "database name"
Assert-Rejected "scheme" "Postgres://fexu@127.0.0.1:5432/fexu" "database name"
Assert-Rejected "exact-name" "postgres://fexu@127.0.0.1:5432/fexu_test" "database name"
Assert-Rejected "remote" "postgres://fexu@203.0.113.10:5432/fexu_test_relocated" "loopback"
Assert-Rejected "encoded-short" "postgres://fexu@127.0.0.1:5432/fexu%5ftest" "database name"
Assert-Rejected "unclear" "postgres://fexu@127.0.0.1:5432/fexu_test_relocated/extra" "single database name"

Assert-Allowed "relocated" "postgres://fexu@127.0.0.1:5432/fexu_test_relocated"
Assert-Allowed "postgresql" "postgresql://fexu@localhost:5432/fexu_test_relocated"
Assert-Allowed "encoded" "postgres://fexu@127.0.0.1:5432/%66exu_test_relocated"
Assert-Allowed "ipv6" "postgres://fexu@[::1]:5432/fexu_test_relocated"

if ($failed -ne 0) {
    exit 1
}
Write-Host "cold-start URL policy passed"
