default:
    @just --list

dev:
    cargo run --locked -- serve

# Rebuild and restart the offline fixture when handwritten source files change.
dev-watch scenario="healthy":
    python3 scripts/dev.py {{quote(scenario)}}

# Run the service against a named local fixture scenario.
demo scenario="healthy":
    cargo run --locked --example dev-fixture -- {{quote(scenario)}}

# Check prerequisites for run, contributor, docs, release, or all workflows.
doctor scope="contributor":
    python3 scripts/doctor.py {{quote(scope)}}

# Install locked project dependencies and verify contributor prerequisites.
setup:
    python3 scripts/doctor.py setup
    pnpm install --frozen-lockfile
    python3 scripts/doctor.py contributor

check:
    just check-doc-links
    just check-frontend
    just check-rust
    just check-python

# Check maintained Markdown links with Python only.
check-doc-links:
    python3 scripts/build-docs.py --check-links

# Check Rust formatting, lints, tests, build, and offline transport smoke coverage.
check-rust:
    cargo fmt --check
    cargo clippy --locked --all-targets -- -D warnings
    cargo test --locked
    cargo build --locked
    python3 scripts/smoke.py --spawn

# Check repository Python tooling, including infrastructure and smoke diagnostics.
check-python:
    python3 -m unittest discover -s scripts -p 'test_*.py'
    python3 -m unittest discover -s infra/aws -p 'test_*.py'

# Check a proposed local commit subject or squash PR title.
check-commit subject:
    python3 scripts/check_commit_subject.py --subject={{quote(subject)}}

# Focused security regressions; these checks are also covered by just check.
check-security:
    python3 -m unittest scripts.test_security
    python3 -m unittest discover -s infra/aws -p 'test_release.py'
    python3 -m unittest discover -s infra/aws -p 'test_dns.py'
    pnpm run lint
    pnpm run test:security
    cargo test --locked --test contract advertised_city_query_bounds_match_runtime_limits

generate-openapi:
    cargo run --locked --example export-openapi -- openapi.json

check-openapi:
    cargo run --locked --example export-openapi -- --check openapi.json

generate-client:
    pnpm run generate

generate: generate-openapi generate-client

check-frontend:
    pnpm run check

# Check compatibility with the minimum Rust version declared by the project.
check-msrv:
    cargo +1.88.0 check --locked --all-targets

# Check advisories, licenses, banned crates, and crate sources with cargo-deny.
audit:
    cargo deny --locked check advisories bans licenses sources

# Build and check the Pages documentation using preinstalled LikeC4 and Playwright browsers.
check-docs:
    #!/usr/bin/env bash
    set -euo pipefail
    likec4 build docs/architecture --output-single-file --use-hash-history -t "Weather Bridge architecture" -o target/likec4
    if [[ -n "${API_URL:-}" ]]; then
        python3 scripts/build-docs.py --architecture target/likec4 --api-url "$API_URL"
    else
        python3 scripts/build-docs.py --architecture target/likec4
    fi
    node scripts/test-docs.cjs
    for view in 'System context' 'Containers' 'Interfaces and transports' 'HTTP server' 'Weather service' 'AWS deployment' 'Building a weather report' 'Building an hourly forecast'; do
        grep -qF "$view" target/docs-site/architecture/index.html || { echo "Missing LikeC4 view: $view"; exit 1; }
    done

# Opt into read-only live NWS checks after changing NWS fetch or parsing behavior.
check-live:
    cargo build --locked
    python3 scripts/smoke.py --spawn --live

# Browser fixtures only; install matching browsers before running.
check-browser:
    pnpm run test:e2e
