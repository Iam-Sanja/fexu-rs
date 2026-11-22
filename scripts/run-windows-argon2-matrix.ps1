# Minimal Windows Argon2 matrix for release acceptance (serial series, 25 runs each).
param(
    [string]$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
)

$ErrorActionPreference = "Stop"
$repro = Join-Path $RepoRoot "scripts\repro-windows-auth-crash"
Push-Location $repro
try {
    cargo build --locked 2>&1 | Out-Null
    cargo build --locked --release 2>&1 | Out-Null
    $rustc = (rustc --version).Trim()
    $argon = (Select-String -Path (Join-Path $repro "Cargo.lock") -Pattern 'name = "argon2"' -Context 0,2 |
        ForEach-Object { if ($_.Context.PostContext -match 'version = "(.+)"') { $Matches[1] } })
    $series = @(
        @{ Label = "debug-8"; Exe = ".\target\debug\fexu-auth-crash-minimal.exe"; Threads = 8 },
        @{ Label = "debug-24"; Exe = ".\target\debug\fexu-auth-crash-minimal.exe"; Threads = 24 },
        @{ Label = "release-8"; Exe = ".\target\release\fexu-auth-crash-minimal.exe"; Threads = 8 },
        @{ Label = "release-24"; Exe = ".\target\release\fexu-auth-crash-minimal.exe"; Threads = 24 }
    )
    $stack = "8388608"
    $results = @()
    foreach ($item in $series) {
        $pass = 0
        $fail = 0
        $av = 0
        for ($i = 1; $i -le 25; $i++) {
            & $item.Exe $item.Threads $stack
            $code = $LASTEXITCODE
            if ($code -eq 0) { $pass++ }
            elseif ($code -eq -1073741819 -or $code -eq 3221225477) { $av++; $fail++ }
            else { $fail++ }
        }
        $results += [pscustomobject]@{
            Series = $item.Label
            Command = "$($item.Exe) $($item.Threads) $stack"
            Runs = 25
            Pass = $pass
            Av = $av
            Fail = $fail
        }
    }
    $out = Join-Path $RepoRoot ".acceptance-argon2-results.json"
    @{
        rustc = $rustc
        argon2 = $argon
        stack = [int]$stack
        results = $results
        recordedAt = (Get-Date).ToString("o")
    } | ConvertTo-Json -Depth 6 | Set-Content -Encoding utf8 $out
    $results | Format-Table -AutoSize
    Write-Host "Wrote $out"
    cargo test -p fexu-auth --lib 2>&1 | Tee-Object -Variable authTest
    if ($LASTEXITCODE -ne 0) { throw "fexu-auth tests failed" }
}
finally {
    Pop-Location
}