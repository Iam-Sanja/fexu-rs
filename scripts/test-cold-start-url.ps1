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
Assert-Rejected "query-hides-name" "postgres://fexu@127.0.0.1:5432/fexu?application_name=fexu_test" "query"
Assert-Rejected "scheme" "Postgres://fexu@127.0.0.1:5432/fexu" "database name"
Assert-Rejected "exact-name" "postgres://fexu@127.0.0.1:5432/fexu_test" "database name"
Assert-Rejected "remote" "postgres://fexu@203.0.113.10:5432/fexu_test_relocated" "loopback"
Assert-Rejected "encoded-short" "postgres://fexu@127.0.0.1:5432/fexu%5ftest" "database name"
Assert-Rejected "unclear" "postgres://fexu@127.0.0.1:5432/fexu_test_relocated/extra" "single database name"
Assert-Rejected "q-dbname-fexu" "postgres://fexu@127.0.0.1:5432/fexu_test_relocated?dbname=fexu" "query"
Assert-Rejected "q-dbname-production" "postgres://fexu@127.0.0.1:5432/fexu_test_relocated?dbname=production" "query"
Assert-Rejected "q-host" "postgres://fexu@127.0.0.1:5432/fexu_test_relocated?host=203.0.113.10" "query"
Assert-Rejected "q-hostaddr" "postgres://fexu@127.0.0.1:5432/fexu_test_relocated?hostaddr=203.0.113.10" "query"
Assert-Rejected "q-port-dbname" "postgres://fexu@127.0.0.1:1/fexu_test_relocated?port=5432&dbname=production" "query"
Assert-Rejected "q-user" "postgres://fexu@127.0.0.1:5432/fexu_test_relocated?user=postgres" "query"
Assert-Rejected "q-password" "postgres://fexu@127.0.0.1:5432/fexu_test_relocated?password=example" "query"
Assert-Rejected "q-options" "postgres://fexu@127.0.0.1:5432/fexu_test_relocated?options=-c" "query"
Assert-Rejected "q-application" "postgres://fexu@127.0.0.1:5432/fexu_test_relocated?application_name=test" "query"
Assert-Rejected "q-empty" "postgres://fexu@127.0.0.1:5432/fexu_test_relocated?" "query"
Assert-Rejected "q-encoded-key" "postgres://fexu@127.0.0.1:5432/fexu_test_relocated?%64bname=fexu" "query"
Assert-Rejected "q-uppercase-key" "postgres://fexu@127.0.0.1:5432/fexu_test_relocated?DBNAME=fexu" "query"
Assert-Rejected "q-options-index" "postgres://fexu@[::1]:5432/fexu_test_relocated?options[search_path]=postgres" "query"
Assert-Rejected "encoded-question-path" "postgres://fexu@127.0.0.1:5432/fexu_test_relocated%3Fdbname=fexu" "database name"
Assert-Rejected "socket-host" "postgres://fexu@%2Fvar%2Frun%2Fpostgresql/fexu_test_relocated" "loopback"
Assert-Rejected "keyword-conninfo" "host=127.0.0.1" "unsupported"
Assert-Rejected "empty-host" "postgres:///fexu_test_relocated" "host"

Assert-Allowed "relocated" "postgres://fexu@127.0.0.1:5432/fexu_test_relocated"
Assert-Allowed "postgresql" "postgresql://fexu@localhost:5432/fexu_test_relocated"
Assert-Allowed "encoded" "postgres://fexu@127.0.0.1:5432/%66exu_test_relocated"
Assert-Allowed "ipv6" "postgres://fexu@[::1]:5432/fexu_test_relocated"
Assert-Allowed "encoded-password" "postgres://fexu:sec%3Fret@127.0.0.1:5432/fexu_test_relocated"

if ($failed -ne 0) {
    exit 1
}
Write-Host "cold-start URL policy passed"
