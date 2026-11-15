#!/usr/bin/env python3
"""Fail-closed crates.io workspace release planner/publisher.

Usage:
  python3 scripts/publish-workspace.py --version 0.1.2 --plan
  python3 scripts/publish-workspace.py --version 0.1.2 --publish

Publishing requires CARGO_REGISTRY_TOKEN, supplied by trusted OIDC publishing.
"""
import argparse
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
USER_AGENT = "fexu-release (https://github.com/Iam-Sanja/fexu-rs)"
INDEX_WAIT_SECONDS = 90
INDEX_POLL_SECONDS = 5


def metadata():
    output = subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=ROOT, text=True,
    )
    return json.loads(output)


def publishable(package):
    setting = package.get("publish")
    return setting is None or "crates-io" in setting


def release_order(version):
    meta = metadata()
    members = set(meta["workspace_members"])
    packages = {p["name"]: p for p in meta["packages"] if p["id"] in members}
    if not packages:
        raise RuntimeError("No workspace packages found")

    public = {name: p for name, p in packages.items() if publishable(p)}
    if not public:
        raise RuntimeError("No publishable crates found")

    bad_versions = sorted(
        f'{name}={p["version"]}' for name, p in public.items()
        if p["version"] != version
    )
    if bad_versions:
        raise RuntimeError(
            f"Release requires version {version} for EVERY publishable crate. "
            "Resolve versions and internal dependency constraints in a separate "
            "reviewed release PR first. Current mismatches: " + ", ".join(bad_versions)
        )

    graph = {}
    for name, package in public.items():
        internal = set()
        for dep in package["dependencies"]:
            if dep.get("path") is None:
                continue
            target = dep["name"]
            if target not in packages:
                raise RuntimeError(f"{name}: unknown path dependency {target}")
            if target not in public:
                raise RuntimeError(f"{name}: nonpublishable path dependency {target}")
            requirement = dep.get("req", "")
            if requirement != f"^{version}" and requirement != version and requirement != f"={version}":
                raise RuntimeError(
                    f"{name} -> {target}: dependency requirement {requirement!r} "
                    f"must be updated to version {version}"
                )
            if target != name:
                internal.add(target)
        graph[name] = internal

    ready = sorted(name for name, deps in graph.items() if not deps)
    result = []
    while ready:
        name = ready.pop(0)
        result.append(name)
        for other, dependencies in graph.items():
            if name in dependencies:
                dependencies.remove(name)
                if not dependencies and other not in result and other not in ready:
                    ready.append(other)
        ready.sort()
    if len(result) != len(public):
        raise RuntimeError("Cycle in publishable workspace dependency graph")
    return result


def index_url(crate):
    name = crate.lower()
    length = len(name)
    if length <= 1:
        prefix = f"1/{name}"
    elif length == 2:
        prefix = f"2/{name}"
    elif length == 3:
        prefix = f"3/{name[0]}/{name}"
    else:
        prefix = f"{name[:2]}/{name[2:4]}/{name}"
    return f"https://index.crates.io/{prefix}"


def index_lists_version(crate, version):
    request = urllib.request.Request(
        index_url(crate),
        headers={"User-Agent": USER_AGENT, "Accept": "text/plain"},
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            body = response.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as exc:
        if exc.code == 404:
            return False
        raise RuntimeError(f"{crate}: crates.io index returned HTTP {exc.code}") from exc
    return f'"vers":"{version}"' in body


def wait_until_indexed(crate, version):
    deadline = time.monotonic() + INDEX_WAIT_SECONDS
    while time.monotonic() < deadline:
        if index_lists_version(crate, version):
            print(f"{crate} {version} is visible in the crates.io index", flush=True)
            return
        time.sleep(INDEX_POLL_SECONDS)
    raise RuntimeError(
        f"{crate} {version} was not visible in the crates.io index within "
        f"{INDEX_WAIT_SECONDS} seconds. Do not publish dependents yet. "
        "A later run skips versions that are already indexed."
    )


def publish_crate(crate, version):
    if index_lists_version(crate, version):
        print(f"{crate} {version} is already indexed; skipping", flush=True)
        return
    print(f"Publishing {crate} {version}", flush=True)
    completed = subprocess.run(
        ["cargo", "publish", "--locked", "--package", crate],
        cwd=ROOT,
        text=True,
    )
    if completed.returncode != 0:
        if index_lists_version(crate, version):
            print(
                f"{crate} {version} is indexed after a publish error; continuing",
                flush=True,
            )
            return
        raise subprocess.CalledProcessError(completed.returncode, completed.args)
    wait_until_indexed(crate, version)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--version", required=True)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--plan", action="store_true")
    mode.add_argument("--publish", action="store_true")
    args = parser.parse_args()

    if not args.version or not all(
        part.isdigit() for part in args.version.split(".")
    ) or len(args.version.split(".")) != 3:
        parser.error("Version must be N.N.N")

    order = release_order(args.version)
    print(f"Release {args.version}: {len(order)} crates in dependency order:", flush=True)
    for index, crate in enumerate(order, 1):
        print(f"  {index:02d}. {crate}", flush=True)
    if args.plan:
        return
    if not os.getenv("CARGO_REGISTRY_TOKEN"):
        raise RuntimeError("Missing trusted-publishing CARGO_REGISTRY_TOKEN")

    # crates.io trusted-publishing tokens expire after 30 minutes.
    # Indexed crates are skipped so a rerun can finish a partial release.
    for crate in order:
        publish_crate(crate, args.version)


if __name__ == "__main__":
    try:
        main()
    except (OSError, subprocess.CalledProcessError, RuntimeError) as exc:
        print(f"Release stopped: {exc}", file=sys.stderr)
        sys.exit(1)
