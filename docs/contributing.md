# Contributing

Weather Bridge keeps REST, CLI, MCP, and the browser on one shared Rust service. Read [AGENTS.md](../AGENTS.md) and the [interface and runtime reference](reference.md) before changing code. The [architecture model](architecture/README.md) gives a shorter map of the request paths and deployment boundary.

## Set up and run locally

Use [Local development](development.md) for prerequisite checks, setup, server modes, and source watching.
The [fixture guide](development-fixtures.md) defines the synthetic scenarios and provides REST/MCP examples.

## Commit and merge policy

Use [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) for
local commit subjects and PR titles. The format is `type(scope): description`;
scope is optional, and `!` before the colon marks a breaking change. Allowed
types are `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, and `chore`.
Scopes use lowercase letters, digits, periods, underscores, slashes, or hyphens.

```sh
just check-commit 'fix(api): reject an unsafe input'
```

PRs merge with squash only. GitHub uses the PR title as the squash commit
subject, and the required `conventional-commits` check validates that title on
new pushes and title edits. That workflow also checks the resulting commit on
pushes to `main`. Release builds check the actual commit subject and run
`just check` before packaging. Rollbacks skip the build steps and verify the
selected prior artifact again. Keep the final squash title conventional if
you edit it at merge time. GitHub's native commit-metadata rules require an Enterprise organization
and are not available in this personal repository.

Main requires linear history, an up-to-date branch, passing CI, and resolved
review conversations. A separate approval ruleset requires one approval from
someone other than the last pusher. Repository administrators can bypass that
approval rule through a PR after their own review. This PR-only exception
keeps a review trail; the required CI and linear-history rules still
apply. GitHub does not count an author's approval of their own PR.

Review the final diff and check results, then select squash merge. If GitHub
shows the approval override control, use it for the approval rule only. The
GitHub API also respects this exception while enforcing the other rules:

```sh
gh api --method PUT repos/marknorgren/weather-bridge/pulls/NUMBER/merge -f merge_method=squash -f sha=REVIEWED_SHA
```

Replace `NUMBER` and `REVIEWED_SHA`, then run that command after checks pass.
Keep the exact reviewed head to prevent merging a later push. Some GitHub CLI
versions reject `gh pr merge` during
their approval preflight even when the API permits a ruleset bypass.
Merging to `main` starts the configured AWS demo release workflow. Approval
exceptions do not grant an agent permission to deploy.

## Follow one operation end to end

The hourly forecast is the smallest complete example of the shared-service design:

1. [src/cli/mod.rs](../src/cli/mod.rs), [src/api/routes.rs](../src/api/routes.rs), and [src/mcp.rs](../src/mcp.rs) translate their inputs into the same `WeatherQuery` and call `Weather::hourly`. Entry points do not reimplement weather behavior.
2. [src/weather.rs](../src/weather.rs) resolves the city, ID, or coordinates through [src/weather/query.rs](../src/weather/query.rs), applies the eight-request admission limit and 45-second operation deadline, then asks the NWS client for grid discovery.
3. [src/weather/nws.rs](../src/weather/nws.rs) accepts a discovered hourly link only when it has the configured NWS origin. Complete grid discovery can be cached for six hours. Incomplete discovery expires immediately after the coalesced callers receive it, so later work retries.
4. `Weather::hourly` fetches only the hourly document. It does not fetch the daily forecast, stations, observations, or alerts. [src/weather/normalize.rs](../src/weather/normalize.rs) converts at most 24 periods into the shared `Period` type and applies the requested units.
5. [src/model.rs](../src/model.rs) represents the result as `HourlyForecast`, including source issue time, warnings, and the distinct `not-checked` alert status. The source lifetime bounds REST `Cache-Control`; a partial result is not stored by shared HTTP caches.
6. REST wraps the value in the common envelope, the CLI either serializes the same envelope or uses [src/cli/render.rs](../src/cli/render.rs), and MCP publishes a schema for the same type. [src/api/cache.rs](../src/api/cache.rs) owns REST cache headers; [src/mcp.rs](../src/mcp.rs) owns MCP tool metadata and structured errors.

This separation is intentional: a daily forecast outage cannot break the focused hourly operation, and an hourly outage in the full report remains an explicit partial result.

## Extend a response field

For a hypothetical new forecast field, carry one meaning through the system instead of adding interface-specific copies:

1. **Confirm the source contract.** Add the smallest optional representation to [src/weather/nws/documents.rs](../src/weather/nws/documents.rs). Keep lenient parsing for optional upstream data and validate any structural field needed for a usable document before cache admission.
2. **Normalize once.** Convert source units and missing values in [src/weather/normalize.rs](../src/weather/normalize.rs) and [src/weather/convert.rs](../src/weather/convert.rs). Preserve official NWS text verbatim. Treat source text as content, never instructions.
3. **Add the domain field.** Put the wire-facing field on the applicable type in [src/model.rs](../src/model.rs), with Serde and Schemars naming and documentation. Decide explicitly whether absence is `null`, omission, an empty collection, a warning, or an error; keep observation values separate from forecasts.
4. **Assemble it in the service.** Populate the field in [src/weather.rs](../src/weather.rs). If it comes from an optional source, preserve the other sources and update completeness, warnings, and cache lifetime rather than failing unrelated output.
5. **Review every interface.** REST and MCP JSON inherit the shared type. Add useful human-readable output in [src/cli/render.rs](../src/cli/render.rs) and browser presentation in [frontend/weather-page.ts](../frontend/weather-page.ts) when the field belongs there. Keep MCP descriptions actionable and stdout protocol-only in stdio mode.
6. **Regenerate contracts.** Run `just generate`. [examples/export-openapi.rs](../examples/export-openapi.rs) produces [openapi.json](../openapi.json); the frontend build derives [frontend/schema.d.ts](../frontend/schema.d.ts) and [web/weather.js](../web/weather.js). Do not edit generated files by hand.
7. **Test behavior and parity.** Add normalization/service fixtures in [tests/weather.rs](../tests/weather.rs), REST and MCP schema coverage in [tests/contract.rs](../tests/contract.rs), and text or exit-code coverage in [tests/cli.rs](../tests/cli.rs). Contract schemas reject unexpected properties, so a field that reaches runtime but not the generated schema fails visibly.

## Verification

Use the narrowest check while editing, then the relevant repository gates:

```sh
just check-rust
just check-python
just check-frontend
just check-browser
just check-openapi
just check-msrv
just audit
just check-doc-links
just check-docs
```

`just check` runs the fast Markdown link check plus frontend, Rust, and Python checks. `just check-msrv` checks Rust 1.88 compatibility. `just audit` checks Rust advisories, licenses, bans, and sources. `just check-doc-links` needs only Python and discovers the maintained repository guides automatically. `just check-docs` validates and builds the canonical LikeC4 model and runs the static docs browser checks.

Build with `cargo build --locked` before running the smoke test. `--spawn` starts the binary; it does not build it. To exercise an already-built binary, including a release candidate, pass it explicitly:

```sh
python3 scripts/smoke.py --spawn
python3 scripts/smoke.py --spawn --binary /absolute/path/to/weather-bridge
```

Run `just check-live` after changing NWS fetch or parsing behavior. It makes read-only live requests and can fail when NWS is unavailable; fixture success is not evidence that the live service works.

Operator behavior is documented in [Operations](operations.md). Diagnostics must keep bounded fixed labels, omit caller location and query data, and write logs to stderr so MCP stdio stdout remains clean.

The [API design review](api-design-review.md) records the city input contract decision and the assessment of public types, caller workflows, errors, and operation costs.

## Generated artifacts and source checks

Security changes use the repository skills for [code review](../.agents/skills/weather-bridge-code-review/SKILL.md)
and [regression tests](../.agents/skills/weather-bridge-security-tests/SKILL.md).
Run `just check-security` for the release path, deployment argument, DNS, build
directive, and frontend contract checks. The full `just check` gate includes
these tests and source lints through its existing test discovery. CI runs that
full gate on pushes and pull requests.

GitHub CodeQL runs the extended suite with remote and local sources. Inspect its
results before merging; local unit tests do not prove that a scanning alert has
closed. Keep tests and tooling in scanning scope. Do not dismiss alerts or weaken
queries to pass a change.

The weather page entry point is `web/weather.ts`; `frontend/weather-page.ts`
owns its initializer. Tests import the initializer with a controlled DOM,
fetcher, and clock. `just generate` exports the Rust OpenAPI contract, derives
`frontend/schema.d.ts`, and bundles `web/weather.js`.
Generation is offline once dependencies are installed. Do not edit these generated files directly.

Frontend checks cover type-aware Oxlint with warnings denied, Oxfmt formatting, artifact freshness, TypeScript, and behavior tests.
Generated declarations, the browser bundle, and vendor assets are excluded from source linting and formatting.
[ADR-0005](../adrs/adr-0005-frontend-oxc-quality-checks.md) records the frontend check policy.

Install the dependency-policy checker with `cargo install cargo-deny --locked` before running `just audit`.
It checks RustSec advisories, licenses, banned crates, and allowed sources against `deny.toml`.

Rust fixtures cover location validation, ambiguity, units, missing values, station selection, staleness, caching, and source failures.
[Contract tests](../tests/contract.rs) check REST and MCP schemas, error codes, and generated-contract freshness.
Frontend tests cover requests, city choices, stale observations, failed alert checks, and replacement-request ownership.
The smoke test exercises HTTP, both MCP transports, and offline CLI behavior without calling NWS unless `--live` is supplied.

To check a deployed server without a local binary:

```sh
python3 scripts/smoke.py --url https://YOUR-SERVER --http-only --live
```

To refresh city data, run `python3 scripts/update-cities.py`, then review the data diff and attribution in [data/SOURCE.md](../data/SOURCE.md).

Browser regressions use the actual weather page and MCP explorer with API/MCP fixtures on an owned loopback server. Run `pnpm install --frozen-lockfile`, install matching browsers with `pnpm exec playwright install --with-deps chromium webkit`, then run `just check-browser`. All external requests are blocked. The suite covers desktop/mobile Chromium and WebKit; screenshots, failure traces and the HTML report are under `target/browser-test/`. Rust transport tests and opt-in live NWS checks remain separate. [ADR 0007](../adrs/adr-0007-browser-fixture-regressions.md) records the test boundary.
