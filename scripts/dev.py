#!/usr/bin/env python3
"""Rebuild and restart the offline development fixture when source files change."""

import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import threading
import time


ROOT = Path(__file__).resolve().parents[1]
GENERATED = {"openapi.json", "frontend/schema.d.ts", "web/weather.js"}
ROOT_INPUTS = {
    "build.rs",
    "tsconfig.json",
    "data/cities.json",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "package.json",
    "pnpm-lock.yaml",
    "pnpm-workspace.yaml",
}
SOURCE_SUFFIXES = {".rs", ".ts", ".js", ".mjs", ".html", ".json"}
SOURCE_DIRECTORIES = ("src", "examples", "tests/common", "frontend", "web")
BUILD_COMMANDS = (
    ("OpenAPI", ["cargo", "run", "--locked", "--example", "export-openapi", "--", "openapi.json"]),
    ("frontend", ["pnpm", "run", "generate"]),
    ("fixture", ["cargo", "build", "--locked", "--example", "dev-fixture"]),
)
STOP_TIMEOUT_SECONDS = 5


def source_snapshot(root=ROOT):
    """Return only handwritten build inputs, excluding generated outputs and build trees."""
    files = []
    for relative in ROOT_INPUTS:
        path = root / relative
        if path.is_file():
            files.append(path)
    for directory in SOURCE_DIRECTORIES:
        base = root / directory
        if not base.is_dir():
            continue
        files.extend(
            path
            for path in base.rglob("*")
            if path.is_file() and path.suffix in SOURCE_SUFFIXES
        )
    snapshot = {}
    for path in files:
        relative = path.relative_to(root).as_posix()
        if relative in GENERATED or relative.startswith(("target/", "node_modules/", "web/vendor/")):
            continue
        try:
            stat = path.stat()
        except FileNotFoundError:
            # Editors commonly replace a file between directory enumeration and stat.
            continue
        snapshot[relative] = (stat.st_mtime_ns, stat.st_size)
    return snapshot


def fixture_binary():
    result = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        timeout=30,
    )
    if result.returncode:
        detail = result.stderr.strip() or result.stdout.strip() or "cargo metadata failed"
        raise RuntimeError(detail)
    try:
        target = Path(json.loads(result.stdout)["target_directory"])
    except (KeyError, TypeError, json.JSONDecodeError) as error:
        raise RuntimeError(f"invalid cargo metadata: {error}") from None
    return target / "debug/examples/dev-fixture"


def stop_process(process, timeout=STOP_TIMEOUT_SECONDS):
    if not process or process.poll() is not None:
        return
    try:
        if os.name == "posix":
            os.killpg(process.pid, signal.SIGTERM)
        else:
            process.terminate()
    except ProcessLookupError:
        process.wait()
        return
    try:
        process.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        try:
            if os.name == "posix":
                os.killpg(process.pid, signal.SIGKILL)
            else:
                process.kill()
        except ProcessLookupError:
            pass
        process.wait()


class DevelopmentLoop:
    def __init__(self, scenario, bind=None, output=sys.stderr, stopping=None):
        self.scenario = scenario
        self.bind = bind
        self.output = output
        self.server = None
        self.build_process = None
        self.failed = False
        self.stopping = stopping or threading.Event()

    def say(self, message):
        print(f"[dev-watch] {message}", file=self.output, flush=True)

    def run_build(self, command):
        self.build_process = subprocess.Popen(
            command,
            cwd=ROOT,
            start_new_session=os.name == "posix",
        )
        try:
            while True:
                try:
                    return self.build_process.wait(timeout=0.1)
                except subprocess.TimeoutExpired:
                    if self.stopping.is_set():
                        stop_process(self.build_process)
                        return None
        finally:
            self.build_process = None

    def build(self):
        for label, command in BUILD_COMMANDS:
            self.say(f"building {label}: {' '.join(command)}")
            status = self.run_build(command)
            if status is None:
                return None
            if status:
                return False
        return True

    def start_server(self):
        binary = fixture_binary()
        environment = os.environ.copy()
        if self.bind:
            environment["WEATHER_BRIDGE_DEV_BIND"] = self.bind
        self.server = subprocess.Popen(
            [str(binary), self.scenario],
            cwd=ROOT,
            env=environment,
            start_new_session=os.name == "posix",
        )
        self.say(f"serving scenario {self.scenario}; waiting for source changes")

    def rebuild(self):
        built = self.build()
        if built is None or self.stopping.is_set():
            return False
        if not built:
            self.failed = True
            self.say("build failed; watching for changes to retry")
            return False
        if self.failed:
            self.say("build recovered")
        self.failed = False
        stop_process(self.server)
        self.start_server()
        return True

    def close(self):
        stop_process(self.build_process)
        self.build_process = None
        stop_process(self.server)
        self.server = None


def changed_files(before, after):
    return sorted(path for path in before.keys() | after.keys() if before.get(path) != after.get(path))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "scenario",
        nargs="?",
        default="healthy",
        choices=("healthy", "stale", "alerts-unavailable"),
    )
    parser.add_argument("--bind", help=argparse.SUPPRESS)
    parser.add_argument("--poll-interval", type=float, default=0.35, help=argparse.SUPPRESS)
    args = parser.parse_args(argv)
    stopping = threading.Event()

    def request_stop(_signum, _frame):
        stopping.set()

    for signal_name in (signal.SIGINT, signal.SIGTERM):
        signal.signal(signal_name, request_stop)

    loop = DevelopmentLoop(args.scenario, bind=args.bind, stopping=stopping)
    snapshot = source_snapshot()
    try:
        loop.rebuild()
        while not stopping.wait(args.poll_interval):
            current = source_snapshot()
            changed = changed_files(snapshot, current)
            if not changed:
                if loop.server and loop.server.poll() is not None:
                    loop.say(f"fixture exited with status {loop.server.returncode}; change a source file to retry")
                    loop.server = None
                    loop.failed = True
                continue
            snapshot = current
            summary = ", ".join(changed[:5])
            if len(changed) > 5:
                summary += f" (+{len(changed) - 5} more)"
            loop.say(f"change detected: {summary}")
            loop.rebuild()
    finally:
        loop.say("stopping")
        loop.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
