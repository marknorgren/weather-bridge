#!/usr/bin/env python3
"""Check Weather Bridge prerequisites without installing tools or reading credentials."""

import argparse
from dataclasses import dataclass
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path


SCOPES = ("run", "setup", "contributor", "docs", "release")
ROOT = Path(__file__).resolve().parents[1]
NODE_REMEDY = "Install Node.js 22.18 or later from https://nodejs.org/en/download"
PNPM_REMEDY = "Install pnpm 11 with: npm install --global pnpm@11; the packageManager field in package.json selects the pinned version"
INSTALL_REMEDY = "From the repository root, run: pnpm install --frozen-lockfile"
LIKEC4_REMEDY = "Install locally: npm install --prefix /tmp/weather-bridge-likec4 --no-save --ignore-scripts likec4@1.59.4; then add /tmp/weather-bridge-likec4/node_modules/.bin to PATH"
PLAYWRIGHT_REMEDY = "Install locally: npm install --prefix /tmp/weather-docs-browser --no-save --ignore-scripts playwright@1.62.1; set NODE_PATH=/tmp/weather-docs-browser/node_modules; then run: node /tmp/weather-docs-browser/node_modules/playwright/cli.js install chromium webkit"
ZIG_REMEDY = "Install locally: python3 -m venv target/dev-tools/zig && target/dev-tools/zig/bin/pip install ziglang==0.15.2 && printf '%s\\n' '#!/bin/sh' 'exec \"$(dirname \"$0\")/python\" -m ziglang \"$@\"' > target/dev-tools/zig/bin/zig && chmod +x target/dev-tools/zig/bin/zig; then add target/dev-tools/zig/bin to PATH"


@dataclass(frozen=True)
class Result:
    name: str
    ok: bool
    detail: str
    remedy: str = ""


def command_result(name, command, remedy):
    executable = command[0]
    if not shutil.which(executable):
        return Result(name, False, f"{executable} was not found", remedy)
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=20)
    except (OSError, subprocess.TimeoutExpired) as error:
        return Result(name, False, str(error), remedy)
    detail = (result.stdout or result.stderr).strip().splitlines()
    return Result(name, result.returncode == 0, detail[0] if detail else "command failed", remedy)


def versioned_result(name, command, required, remedy, minimum=False):
    result = command_result(name, command, remedy)
    if not result.ok:
        return result
    match = re.search(r"(?<![0-9])(\d+)\.(\d+)\.(\d+)(?![0-9])", result.detail)
    found = tuple(map(int, match.groups())) if match else None
    expected = tuple(map(int, required.split(".")))
    valid = found is not None and (found >= expected if minimum else found == expected)
    if valid:
        return result
    relation = "or later" if minimum else "exactly"
    return Result(name, False, f"found {result.detail}; need {relation} {required}", result.remedy or remedy)


def run_checks():
    remedy = "Install rustup from https://rustup.rs, then run: rustup toolchain install"
    return [
        command_result("cargo", ["cargo", "--version"], remedy),
        command_result("rustc", ["rustc", "--version"], remedy),
    ]


def node_check():
    return versioned_result(
        "node",
        ["node", "--version"],
        "22.18.0",
        NODE_REMEDY,
        minimum=True,
    )


def pnpm_check():
    return versioned_result("pnpm", ["pnpm", "--version"], "11.0.0", PNPM_REMEDY, minimum=True)


def setup_checks():
    return run_checks() + [
        node_check(),
        pnpm_check(),
        command_result("just", ["just", "--version"], "Install Just using https://just.systems/man/en/packages.html"),
    ]


def locked_versions(lockfile):
    """Return direct dependency versions of the root importer in a pnpm v9 lockfile.

    The lockfile is YAML, but pnpm writes this section in a fixed block layout, so a
    line reader avoids a third-party YAML dependency. Peer suffixes such as
    "7.13.0(typescript@5.9.3)" are dropped to leave the installed package version.
    """
    sections = ("dependencies:", "devDependencies:", "optionalDependencies:")
    versions = {}
    in_root = in_section = False
    name = None
    for line in lockfile.splitlines():
        if not line.strip():
            continue
        indent = len(line) - len(line.lstrip(" "))
        text = line.strip()
        if indent == 0:
            in_root = in_section = False
        elif indent == 2:
            in_root, in_section = text == ".:", False
        elif in_root and indent == 4:
            in_section, name = text in sections, None
        elif in_section and indent == 6 and text.endswith(":"):
            name = text[:-1].strip("'\"")
        elif in_section and indent == 8 and name and text.startswith("version:"):
            version = text.split(":", 1)[1].strip().strip("'\"")
            versions[name] = version.split("(", 1)[0]
    return versions


def frontend_dependencies(root=ROOT):
    """Check direct frontend package versions against the committed lockfile."""
    remedy = INSTALL_REMEDY
    try:
        manifest = json.loads((root / "package.json").read_text())
        locked = locked_versions((root / "pnpm-lock.yaml").read_text())
        names = manifest.get("dependencies", {}) | manifest.get("devDependencies", {})
        invalid = []
        for name in names:
            package = root / "node_modules" / name / "package.json"
            try:
                installed = json.loads(package.read_text())
                expected = locked[name]
                bins = installed.get("bin", {})
                if isinstance(bins, str):
                    bins = {name.rsplit("/", 1)[-1]: bins}
                if installed.get("version") != expected or any(
                    not (root / "node_modules/.bin" / command).is_file() for command in bins
                ):
                    invalid.append(name)
            except (OSError, ValueError, KeyError, TypeError):
                invalid.append(name)
    except (OSError, ValueError, KeyError, TypeError) as error:
        return Result("frontend dependencies", False, f"cannot check package metadata: {error}", remedy)
    detail = "missing or outdated packages: " + ", ".join(invalid) if invalid else "direct frontend packages match the lockfile"
    return Result("frontend dependencies", not invalid, detail, remedy)


def contributor_checks():
    return setup_checks() + [frontend_dependencies()]


def docs_checks():
    node = node_check()
    likec4 = versioned_result(
        "likec4", ["likec4", "--version"], "1.59.4", LIKEC4_REMEDY
    )
    playwright = versioned_result(
        "playwright",
        ["node", "-p", "require('playwright/package.json').version"],
        "1.62.1",
        PLAYWRIGHT_REMEDY,
    )
    browser_check = "const {chromium,webkit}=require('playwright');const fs=require('fs');const ok=[chromium,webkit].every(b=>fs.existsSync(b.executablePath()));if(ok)console.log('Chromium and WebKit installed');process.exit(ok?0:1)"
    browsers = command_result(
        "playwright browsers",
        ["node", "-e", browser_check],
        PLAYWRIGHT_REMEDY,
    )
    return [node, likec4, playwright, browsers]


def release_checks():
    return [
        versioned_result(
            "cargo-lambda",
            ["cargo", "lambda", "--version"],
            "1.9.2",
            "Install locally: cargo install --root target/dev-tools cargo-lambda --version 1.9.2 --locked; then add target/dev-tools/bin to PATH",
        ),
        versioned_result(
            "zig",
            ["zig", "version"],
            "0.15.2",
            ZIG_REMEDY,
        ),
        command_result("git", ["git", "--version"], "Install Git from https://git-scm.com/downloads"),
        command_result("gh", ["gh", "--version"], "Install GitHub CLI from https://cli.github.com"),
        command_result("aws", ["aws", "--version"], "Install AWS CLI v2 from https://docs.aws.amazon.com/cli/latest/userguide/getting-started-install.html"),
        versioned_result("terraform", ["terraform", "version"], "1.15.8", "Install Terraform 1.15.8 using https://developer.hashicorp.com/terraform/install"),
    ]


def check_scope(scope):
    checks = {
        "run": run_checks,
        "setup": setup_checks,
        "contributor": contributor_checks,
        "docs": docs_checks,
        "release": release_checks,
    }
    if scope not in checks:
        raise ValueError(f"unknown prerequisite scope: {scope}")
    return checks[scope]()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scope", nargs="?", choices=(*SCOPES, "all"), default="contributor")
    args = parser.parse_args(argv)
    scopes = SCOPES if args.scope == "all" else (args.scope,)
    failed = False
    for scope in scopes:
        print(f"{scope}:")
        for result in check_scope(scope):
            status = "ok" if result.ok else "missing"
            print(f"  [{status}] {result.name}: {result.detail}")
            if not result.ok:
                failed = True
                print(f"    remedy: {result.remedy}")
    if failed:
        print("No credentials were read or changed.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
