# Interface and runtime reference

Read this guide for CLI, REST, MCP, configuration, caching, coverage, and resource limits. Start with the [README](../README.md) to run the service.

## CLI

```sh
cargo run --locked -- weather "Seattle, WA"
cargo run --locked -- weather "Seattle, WA" --units metric --json
cargo run --locked -- cities "Springfield"
```

| Command | Output |
| --- | --- |
| `weather <LOCATION>` | Observation, forecast summary and alerts. |
| `hourly <LOCATION>` | Next 24 hours, one line each. Does not check alerts. |
| `alerts <LOCATION>` | Active alerts with the original NWS instructions, or a note that the check failed. |
| `cities <QUERY>` | Offline city search, printed as a table. |

`<LOCATION>` is one of:

- a city: `"Seattle, WA"`
- a city ID: `--city-id <ID>`
- coordinates: `--lat <LAT> --lon <LON>`

Any other combination is a usage error. `--units us|metric` picks units. `--json` prints the same `{ "data", "meta" }` envelope as the REST API.

"Alerts could not be checked" means NWS failed. It never means there are no alerts.

Errors print to stderr, or to stdout as JSON with `--json`. An ambiguous or unknown city lists numbered choices, such as `Franklin, TN (id 4623560)`. Rerun with `--city-id` or `"City, ST"`.

| Exit code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | Upstream or other failure (NWS unavailable, timeout, busy, internal error) |
| 2 | Usage error, invalid input, unsupported location, city not found or ambiguous |
| 3 | Output printed, but incomplete: alerts could not be checked or a source is missing |

When a pipe reader closes stdout early, the CLI exits quietly and preserves the command's exit code. Other output failures exit 1.

## REST API

```sh
curl 'http://127.0.0.1:8790/v1/weather?city=Seattle%2C%20WA'
curl 'http://127.0.0.1:8790/v1/cities?q=Springfield'
curl 'http://127.0.0.1:8790/v1/weather?cityId=5809844&units=metric'
curl 'http://127.0.0.1:8790/v1/weather?lat=47.6062&lon=-122.3321'
curl 'http://127.0.0.1:8790/v1/forecast/hourly?city=Seattle%2C%20WA'
curl 'http://127.0.0.1:8790/v1/alerts?city=Seattle%2C%20WA'
```

The full contract is [openapi.json](../openapi.json), also served at `/openapi.json`.

The contract is generated from Rust routes and wire types with Aide; do not edit it directly. Run `just generate` after API or weather-page changes to update the contract, TypeScript client types and browser bundle. [ADR-0003](../adrs/adr-0003-generate-openapi-from-rust.md) records contract ownership, and [ADR-0004](../adrs/adr-0004-typed-weather-browser-client.md) records the typed browser client.

### Locations

Pass exactly one of `city`, `cityId`, or `lat` with `lon`.

- City names and search prefixes must contain 2 to 120 Unicode characters after trimming surrounding whitespace. REST and MCP schemas use a pattern for these bounds that counts Unicode characters with or without the JavaScript Unicode regex flag. This checks length, not whether a city exists.
- City names accept state abbreviations or full names: `Seattle, Washington`.
- City lookup needs an exact name. `/v1/cities?q=Seat` returns prefix suggestions.
- When several places share a name, the API never guesses. `Springfield` returns 409 `AMBIGUOUS_CITY` with every exact match, sorted by population, then ID. Pick one by `cityId`.

| Status | Cause |
| --- | --- |
| 400 | Invalid request input: unknown or malformed query parameters, an invalid location or an unreadable request body. All use `INVALID_LOCATION`. |
| 404 | Unknown city, with suggestions when available |
| 409 | Ambiguous city, with choices |
| 422 | Coordinates outside NWS coverage |

### Responses

Success: `{ "data": ..., "meta": ... }`, with attribution in `meta`. Errors: `{ "errors": [{ "status", "code", "detail", "choices" }] }`.

- `current` is a station observation or null. It is never a forecast. It includes `observedAt`, `ageSeconds`, `stale`, station distance and source URL. Observations older than two hours are marked stale. Missing values stay null.
- `forecast` has up to 14 NWS periods; `hourly` has up to 24. Numeric temperature and wind fields use the selected units. NWS forecast text is kept word for word, so numbers inside it stay in NWS units.
- `location.latitude` and `longitude` are the city center or your coordinates. `location.gridLookupPoint` is the point sent to the NWS `/points` grid lookup, rounded to two decimals (about 1 km) so nearby requests share it.
- Alerts use four-decimal coordinate precision rather than the two-decimal forecast lookup point. Rounding can still shift a point near a warning boundary.
- `alertsStatus: "unavailable"` means alerts could not be checked, including malformed collections or entries. With `alertsStatus: "checked"`, an empty alert list means none are active. With `alertsStatus: "unavailable"`, an empty list means the result is unknown. An NWS response without a `features` list also counts as a failed check. Original alert instructions are kept.
- `assembledAt` is when the report was built, not when the data were observed. Forecast sources include their issue time when NWS provides one; observations include `observedAt`. Alert sources identify the query URL.
- A missing or invalid hourly or station source leaves the rest of a weather report available with a warning. `sources.hourly` is null when NWS did not provide a safe hourly source URL.

REST requests with bodies larger than 16 KiB return 413 `REQUEST_TOO_LARGE`. Weather lookup and HTTP request deadlines return 504 `UPSTREAM_TIMEOUT`. More than eight weather lookups at once, or more uncached lookups than the upstream request pace can start in time, return 503 `BUSY` within seconds; retry shortly. When the pace cannot start an optional source in time, the report keeps that source's warning instead. All of these use the error envelope and `Cache-Control: no-store`.

`/v1/forecast/hourly` and `/v1/alerts` fetch only what they need and share the report cache. Alerts don't depend on the grid lookup or forecasts, so a forecast outage doesn't hide alerts. The hourly endpoint reports `alertsStatus: "not-checked"`.

The browser automatically refreshes visible reports within the two-minute freshness window. Expired reports and alert checks are labeled when a refresh fails; hidden tabs refresh on return.

`cacheMaxAgeSeconds` is the upstream cache ceiling, not the remaining lifetime of this response. Use HTTP `Cache-Control` for its remaining lifetime.

Forecasts, observations and alerts are cached for up to 120 seconds. Grid lookups (`/points`, which map coordinates to an NWS forecast office and grid) are cached for up to six hours. The service never serves an expired entry as fresh.

Documents must contain the required fields before they can be reused from the cache. Incomplete grid discovery remains usable for the sources it provides, but is retried on the next lookup so a temporary missing link does not last for the six-hour cache window.

## MCP

Build with `cargo build --locked`, then point your MCP client at the binary:

```json
{
  "mcpServers": {
    "weather-bridge": {
      "command": "/absolute/path/to/weather-bridge/target/debug/weather-bridge",
      "args": ["mcp"]
    }
  }
}
```

In stdio mode, logs go to stderr and stdout carries only MCP messages. `serve` also exposes Streamable HTTP at `http://127.0.0.1:8790/mcp`. It accepts MCP 2026-07-28 and 2025-11-25 clients and keeps no sessions.

| Tool | Returns |
| --- | --- |
| `search_cities` | Matching cities by name or prefix, with IDs, states and coordinates |
| `get_weather` | Observation, summary, forecast, alerts, sources and warnings |
| `get_hourly_forecast` | Next 24 hours with source details |
| `get_active_alerts` | NWS alerts and whether the check succeeded |

Weather tools take `city`, `cityId`, or `lat`/`lon`, plus optional `units` (`us` or `metric`). `search_cities` takes `query`.

- Ambiguous cities and source failures return tool errors with details the agent can act on.
- Each tool publishes an `outputSchema`: the `{ "data", "meta" }` envelope on success, or `{ "errors" }` when `isError` is true. Results include matching structured content.
- All tools are marked read-only.
- Forecast and alert text is data from NWS, not instructions to an agent.

## Configuration

`serve` reads configuration once at startup. Every flag has a matching environment variable. An invalid value stops startup with an error; the `--origin-verify` error never shows the value.

| Flag | Environment variable | Default | Purpose |
| --- | --- | --- | --- |
| `--user-agent` | `WEATHER_BRIDGE_USER_AGENT` | Generic project string | Contact User-Agent sent to NWS. Public deployments must set one. |
| `--docs-url` | `WEATHER_BRIDGE_DOCS_URL` | Project GitHub Pages site | HTTPS base URL of your docs, including any repository subpath. No credentials, query or fragment. A trailing slash is added if missing. |
| `--bind` | `WEATHER_BRIDGE_BIND` | `127.0.0.1:8790` | Listen address. |
| `--mcp-hosts` | `WEATHER_BRIDGE_MCP_HOSTS` | Loopback only | Comma-separated trusted MCP `Host` values. |
| `--mcp-origins` | `WEATHER_BRIDGE_MCP_ORIGINS` | Loopback only | Comma-separated trusted MCP `Origin` values. |
| `--origin-verify` | `WEATHER_BRIDGE_ORIGIN_VERIFY` | Unset | Shared value CloudFront sends as `X-Weather-Bridge-Origin-Verify`. At least 32 visible ASCII characters. |

The NWS base URL is a library setting, not a flag. `weather::Config::nws_base_url` defaults to `https://api.weather.gov` and must be a plain `http` or `https` origin. All requests go to it, links found in NWS grid lookups must point to it, and source URLs in responses name it. Tests point it at a loopback fixture.

### Origin check

With `--origin-verify` set, every request must send `X-Weather-Bridge-Origin-Verify` exactly once with exactly that value. The only exception is the Lambda Web Adapter readiness check: `GET`/`HEAD /healthz` with no query string and a loopback Host (`127.0.0.1:8080`, `localhost:8080` or `[::1]:8080`).

- The comparison runs in constant time.
- Other requests get 403, `Cache-Control: no-store`, and a generic `{"errors": [...]}` body that doesn't name the header.
- The value is never logged, and `--help` hides it.

Leave it unset for local runs. The AWS deployment sets it; see [Origin protection](../infra/aws/README.md#origin-protection).

## Security headers

Every response sends `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Strict-Transport-Security` (ignored over plain-HTTP localhost) and a Content-Security-Policy.

- HTML pages allow only same-origin scripts and `fetch`, and can't be framed. There are no inline scripts; page scripts live under `/assets/`.
- JSON and MCP responses get `default-src 'none'`.
- Text responses are gzip- or Brotli-compressed when the client sends `Accept-Encoding`.

## HTTP caching and CORS

`Cache-Control` lets CloudFront and browsers cache only what is safe to reuse:

| Response | `Cache-Control` |
| --- | --- |
| 200 from `/v1/weather`, `/v1/forecast/hourly`, `/v1/alerts` | `public, max-age=N, s-maxage=N`. N is the remaining life of the oldest source used, at most 120 s, so cached data never outlives the two-minute freshness window. |
| Same, with `alertsStatus: "unavailable"` or a partial-source warning | `no-store` |
| 200 from `/v1/cities` | `public, max-age=86400` |
| 400, 404 and 409 errors | `public, max-age=60` |
| 422, 429, 5xx, `BUSY` and timeouts | `no-store` |
| HTML pages, page scripts and `/openapi.json` | `public, max-age=300` |
| `/healthz`, `/version`, `/metrics`, `/mcp`, the origin-check 403, and unknown paths | `no-store` |

The hourly endpoint always includes a note that it doesn't check alerts. That note doesn't trigger `no-store`; any other warning does.

CORS:

- `GET` and `HEAD` on `/v1/*` and `/openapi.json` send `Access-Control-Allow-Origin: *`. This lets the Pages REST reference, and any other browser app, call the API.
- `OPTIONS` on `/v1/*` answers preflight with `GET, HEAD`, the `Accept` header, and a one-day max-age.
- Credentials are never allowed. A wildcard is safe because the REST API is public, read-only and uses no cookies.
- `/mcp`, pages and `/healthz` send no CORS headers. `/mcp` keeps its Origin allowlist.

## Limits and coverage

Coverage matches NWS: the US and its supported territories. The GeoNames index includes places with more than 1,000 people and some administrative seats, not every settlement. Coordinates work for anywhere NWS covers.

A city forecast is for the city center, not a street address. The nearest weather station may be several kilometers away.

| Limit | Value |
| --- | --- |
| Concurrent weather lookups | 8 |
| Upstream request starts | Burst of 6 (one uncached report), then 1 per second per process |
| Wait for an upstream start | Until 6 seconds after the lookup began, oldest lookup first; then `BUSY` |
| Upstream fetch timeout | 12 seconds |
| Observation search, across up to three stations | 12 seconds, then the report has a warning and no observation |
| Report deadline | 45 seconds |
| Shutdown drain | 5 seconds |
| Forecast, observation and alert cache | 2 minutes |
| Grid lookup cache | 6 hours |
| Upstream response size | 2 MB |

Upstream failures:

- Only a 404 from the NWS grid lookup, or an alert point outside NWS bounds, returns `OUTSIDE_COVERAGE`.
- Other NWS 4xx and 5xx failures return `UPSTREAM_UNAVAILABLE` or a partial-source warning.
- Each failure is logged to stderr at `WARN` with the upstream path and cause: transport error, HTTP status, oversized body, invalid JSON or missing required field. The query string is left out because it holds the caller's coordinates. Clients get only a generic message.
- An NWS field that is missing or has the wrong type becomes `null`. A temperature or wind speed in a unit the service does not convert also becomes `null` rather than being guessed. A missing required part, such as forecast periods, fails the request. Each alert entry must contain a properties object with non-empty event text. Its identifier and descriptive fields, including instructions and timestamps, must be strings or null when supplied; a wrong type makes the whole alert check unavailable.

There is no uptime guarantee.

## Public hosting

The default bind address is loopback. A public deployment needs:

- a contact User-Agent (`WEATHER_BRIDGE_USER_AGENT`)
- TLS and gateway rate limits; the demo has no authentication
- its own trusted MCP hosts and origins (`WEATHER_BRIDGE_MCP_HOSTS`, `WEATHER_BRIDGE_MCP_ORIGINS`)
- a review of resource limits; replicas don't share the in-memory upstream limiter

The [AWS deployment and usage guard](../infra/aws/README.md) packages the demo for Lambda with fixed MCP hosts and origins, reserved concurrency, account-wide usage checks and a demo-only shutdown. Release artifacts are built and verified independently; AWS deployment is disabled unless the canonical repository's `AWS_DEPLOY_ENABLED` variable is exactly `true`. The deployment verifies the release manifest locally before any AWS command. The [hosting ADR](../adrs/adr-0001-public-demo-hosting.md) compares Cloudflare and AWS options, free allowances and trials.
