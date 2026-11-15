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
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


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

    for index, crate in enumerate(order):
        print(f"Publishing {crate} {args.version}", flush=True)
        subprocess.run(
            ["cargo", "publish", "--locked", "--package", crate], cwd=ROOT, check=True
        )
        if index != len(order) - 1:
            # Give crates.io and the registry index time to expose new dependencies.
            time.sleep(30)


if __name__ == "__main__":
    try:
        main()
    except (OSError, subprocess.CalledProcessError, RuntimeError) as exc:
        print(f"Release stopped: {exc}", file=sys.stderr)
        sys.exit(1)
