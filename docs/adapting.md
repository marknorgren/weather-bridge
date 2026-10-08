# Adapting Weather Bridge

Weather Bridge can be used as a service starter, but a fork must replace project identity and deliberately choose which weather-specific behavior to retain. Work through this checklist before publishing a renamed service.

## Identity and public contract

- Rename the Cargo package, binary, crate imports, CLI help, MCP server name and instructions, browser titles, OpenAPI title, artifact names, and infrastructure resource prefixes. Search for both `weather-bridge` and `weather_bridge`.
- Replace the repository, Pages, demo, source, contact, and license URLs. The current defaults include `github.com/marknorgren/weather-bridge`, `marknorgren.github.io/weather-bridge`, and `bridge.wx.mrkd.co`.
- Choose a new environment-variable prefix and update Clap bindings, deployment templates, workflows, examples, and docs together. Current runtime names use `WEATHER_BRIDGE_*`; release builds also use `WEATHER_BRIDGE_BUILD_REVISION`.
- Rename HTTP headers and metric names if the service identity changes. Preserve the request-ID validation rules and fixed metric label sets, or document and test the new bounded sets.
- Regenerate [openapi.json](../openapi.json), [frontend/schema.d.ts](../frontend/schema.d.ts), and [web/weather.js](../web/weather.js) after contract or identity changes.

## Domain and source behavior

- Replace [src/model.rs](../src/model.rs) and [src/weather.rs](../src/weather.rs) around the new domain while keeping entry points thin. REST, CLI, MCP, and the browser should still call one shared service.
- Review all Weather-specific modules: NWS documents, normalization, units, alert semantics, observation staleness, city resolution, cache lifetime, upstream limits, fixed-origin link checks, and forecast coverage.
- Replace or remove the GeoNames index and [data/SOURCE.md](../data/SOURCE.md) only with a documented source and license. The current city data is CC BY 4.0 even though the code is MIT. Preserve attribution in responses and distributed artifacts while that data remains.
- Replace NWS attribution and the contact `User-Agent` when changing providers. Reassess provider terms, geographic coverage, rate limits, redirects, response-size limits, cache rules, required fields, and partial-result semantics.
- Keep external source data as content. Do not turn forecasts, alerts, or provider text into agent instructions. Preserve original alert instructions when NWS remains a source.
- Remove claims and tests for unimplemented capabilities. This repository does not ingest radar, use a database, authenticate callers, or provide an uptime guarantee.

## Runtime and diagnostics

- Review every default in [src/cli/args.rs](../src/cli/args.rs): bind address, docs URL, trusted MCP hosts and origins, origin verification, and the NWS contact user agent.
- Keep `/healthz` limited to process liveness. Decide whether `/version` and `/metrics` are public for the new deployment, then align the router, gateway cache policy, monitoring, and [operations guide](operations.md).
- Set `WEATHER_BRIDGE_BUILD_REVISION` only while compiling a release, using the exact 40-character lowercase Git revision. Local builds deliberately report `unknown`. The embedded value appears in CLI version output, `/version`, startup logs, and build metrics.
- Keep request logs free of raw URIs, query strings, coordinates, headers, and credentials. Local logs are readable text; the Lambda environment uses JSON. `RUST_LOG` controls filtering.

## Documentation and architecture

- Update [README.md](../README.md), the [runtime reference](reference.md), [contributing.md](contributing.md), the static [docs site](site/README.md), ADRs, and the canonical [LikeC4 model](architecture/weather-bridge.c4). Do not create a second manually maintained diagram of the same architecture.
- Replace examples and fixture labels. Keep offline demo data unmistakably synthetic, and include healthy, stale, and upstream-unavailable states appropriate to the new domain.
- Review bundled browser assets and their notices. Keep third-party licenses beside distributed assets.

## Release and deployment

- Treat GitHub Pages and AWS as separate destinations. Set the Pages API URL to the fork's HTTPS origin and pass the matching docs URL to the server.
- Deployment is default-off. The [release workflow](../.github/workflows/release.yml) builds and verifies an immutable artifact, but its AWS job runs only in the canonical repository on `main` when the repository variable `AWS_DEPLOY_ENABLED` is exactly `true`.
- Review the `weather-demo` environment variables, OIDC role, account allowlist, region, contact, concurrency, CloudFront plan, domain, origin-verification header, alarms, budget, and usage guard in [infra/aws/README.md](../infra/aws/README.md). Resource names and cost assumptions belong to this demo and must be re-evaluated for a fork.
- Verify a downloaded or locally staged release before any AWS command:

  ```sh
  python3 scripts/release.py verify \
    --release /path/to/release-directory \
    --revision 0123456789abcdef0123456789abcdef01234567
  ```

  [scripts/release.py](../scripts/release.py) checks the manifest revision, target, file set, sizes, SHA-256 digests, embedded binary revision, Lambda wrapper, and guard content. [infra/aws/deploy.py](../infra/aws/deploy.py) repeats verification before it initializes Terraform or calls AWS.
- Keep `/mcp`, `/healthz`, `/version`, and `/metrics` uncached at the gateway. Review REST cache forwarding whenever the public host or provider changes.
- Run [scripts/smoke.py](../scripts/smoke.py) against the exact candidate binary before enabling deployment. Publishing, credentials, DNS, and remote resource creation require a separate explicit operational decision.
