#!/usr/bin/env bash
# Repeatable Fexu baseline. Does not start PostgreSQL and does not build other frameworks.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
runs=${1:-3}
case "$runs" in
  ''|*[!0-9]*) echo "run count must be an integer" >&2; exit 2 ;;
esac
if [ "$runs" -lt 1 ]; then
  echo "run count must be at least 1" >&2
  exit 2
fi
stamp=$(date -u +%Y%m%d-%H%M%S)
dest="$root/examples/bench-baseline/runs/$stamp"
mkdir -p "$dest"

cpu="not measured"
cores="not measured"
memory_kb="not measured"
if [ -r /proc/cpuinfo ]; then
  cpu=$(awk -F: '/model name/ { gsub(/^[ \t]+/, "", $2); print $2; exit }' /proc/cpuinfo)
  cores=$(awk -F: '/cpu cores/ { gsub(/^[ \t]+/, "", $2); print $2; exit }' /proc/cpuinfo)
fi
if [ -r /proc/meminfo ]; then
  memory_kb=$(awk '/MemTotal/ { print $2; exit }' /proc/meminfo)
fi
cat > "$dest/host.json" <<EOF
{
  "schema": 1,
  "collected_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "os": "$(uname -s)",
  "os_release": "$(uname -r)",
  "architecture": "$(uname -m)",
  "cpu": "$cpu",
  "cpu_cores": "$cores",
  "memory_kb": "$memory_kb",
  "rustc": "$(rustc -V)",
  "git_commit": "$(git rev-parse HEAD)",
  "profile": "release",
  "runs": $runs,
  "computer_name_recorded": false,
  "user_name_recorded": false
}
EOF

start=$(date +%s%N)
cargo build --release -p fexu-bench-baseline
end=$(date +%s%N)
cat > "$dest/build.json" <<EOF
{
  "schema": 1,
  "command": "cargo build --release -p fexu-bench-baseline",
  "elapsed_ns": $((end - start)),
  "incremental": true,
  "note": "Wall time of this process. A warm target directory makes this shorter than a clean build. This is not a clean-build claim."
}
EOF

bin="$root/target/release/fexu-bench-baseline"
index=1
while [ "$index" -le "$runs" ]; do
  "$bin" --data "$dest/data-$index" --out "$dest/run-$index.json"
  index=$((index + 1))
done
printf '%s\n' "$dest"
