# Repeatable Fexu baseline. Does not start PostgreSQL and does not build other frameworks.
$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $Root

$Runs = 3
if ($args.Count -ge 1) {
    $Runs = [int]$args[0]
    if ($Runs -lt 1) { throw "run count must be at least 1" }
}

$Stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$Dest = Join-Path $Root "examples\bench-baseline\runs\$Stamp"
New-Item -ItemType Directory -Force -Path $Dest | Out-Null

$Cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$Os = Get-CimInstance Win32_OperatingSystem
$HostInfo = [ordered]@{
    schema = 1
    collected_at = (Get-Date).ToUniversalTime().ToString("o")
    os_caption = $Os.Caption
    os_version = $Os.Version
    architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    cpu = $Cpu.Name
    cpu_cores = $Cpu.NumberOfCores
    cpu_logical_processors = $Cpu.NumberOfLogicalProcessors
    cpu_max_mhz = $Cpu.MaxClockSpeed
    memory_bytes = [int64](Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory
    rustc = (rustc -V | Out-String).Trim()
    git_commit = (git rev-parse HEAD).Trim()
    profile = "release"
    runs = $Runs
    computer_name_recorded = $false
    user_name_recorded = $false
}
$HostInfo | ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $Dest "host.json")

$Build = Measure-Command { cargo build --release -p fexu-bench-baseline }
$BuildInfo = [ordered]@{
    schema = 1
    command = "cargo build --release -p fexu-bench-baseline"
    elapsed_ns = [int64]($Build.Ticks * 100)
    incremental = $true
    note = "Wall time of this process. A warm target directory makes this shorter than a clean build. This is not a clean-build claim."
}
$BuildInfo | ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $Dest "build.json")

$Binary = Join-Path $Root "target\release\fexu-bench-baseline.exe"
if (-not (Test-Path $Binary)) { throw "missing $Binary" }

for ($Index = 1; $Index -le $Runs; $Index++) {
    $Data = Join-Path $Dest "data-$Index"
    $Out = Join-Path $Dest "run-$Index.json"
    & $Binary --data $Data --out $Out
    if ($LASTEXITCODE -ne 0) { throw "run $Index failed" }
}

Write-Output $Dest
