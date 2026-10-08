# Architecture

[weather-bridge.c4](weather-bridge.c4) is the canonical architecture model. Its views cover the system context, containers, HTTP stack, shared weather service, AWS deployment, full report assembly, and the hourly path. The [published documentation](https://marknorgren.github.io/weather-bridge/architecture/) renders this model interactively.

The model follows the code rather than defining a second contract:

- [src/api/mod.rs](../../src/api/mod.rs) owns HTTP middleware order; [src/api/routes.rs](../../src/api/routes.rs) owns routes and operator endpoints.
- [src/weather.rs](../../src/weather.rs) coordinates location resolution and source fetches. [src/weather/nws.rs](../../src/weather/nws.rs) owns the fixed upstream origin, caches, limits, and validation.
- [src/model.rs](../../src/model.rs) owns shared REST, CLI, and MCP response types. [src/weather/normalize.rs](../../src/weather/normalize.rs) maps NWS documents into those types.
- [src/diagnostics.rs](../../src/diagnostics.rs) owns fixed diagnostic labels and process-local metrics.
- [infra/aws/README.md](../../infra/aws/README.md) owns deployment behavior and safeguards.

## Where MCP sits

MCP sits behind the HTTP server and beside REST. It is not a layer between REST and the weather logic.

```text
Browser, HTTP client ──► HTTP server ──► REST routes ─────────┐
AI agent over HTTP ────► HTTP server ──► /mcp ──► MCP tools ──┤
AI agent over stdio ────────────────────────────► MCP tools ──┼──► Weather service
Person in a terminal ───────────────────────────► CLI ────────┘
```

- Over HTTP, `/mcp` is one route in the Axum app. MCP requests pass through the same middleware as REST requests, including origin checks, size limits and the timeout. Then rmcp's Streamable HTTP transport creates a stateless `WeatherMcp` for each request.
- Over stdio, `weather-bridge mcp` runs the same MCP tools without HTTP.
- REST handlers, MCP tools and the CLI are peer adapters. Each one calls `Weather` directly, so all three return the same data and errors.

The `interfaces` view in the model shows this layout.

## Code layout

Built with Axum, Tokio, Serde, Clap, Reqwest/rustls, thiserror/anyhow, tracing, **rmcp**, **moka**, **rstar** and **unicode-normalization**.

- REST, CLI and MCP call the same service.
- Moka merges duplicate in-flight fetches and caches successful responses.
- Rstar indexes city positions on a unit sphere.
- Unicode normalization lets `Rio Grande` match `Río Grande`.
- The demo stores nothing, so there is no database.

Entry points: [src/main.rs](../../src/main.rs), [src/weather.rs](../../src/weather.rs), [src/cities.rs](../../src/cities.rs), [src/api/](../../src/api/mod.rs), [src/mcp.rs](../../src/mcp.rs), and the embedded [web/index.html](../../web/index.html). Response types live in [src/model.rs](../../src/model.rs).

`src/weather.rs` holds configuration and builds the report, hourly and alerts results. The rest of the weather service is in `src/weather/`:

| Module | Job |
| --- | --- |
| `query.rs` | `WeatherQuery`, `Units`, `GridPoint` and location resolution |
| `nws.rs` | NWS HTTP client: fixed origin, size-capped fetches, failure logging, document and grid-lookup caches |
| `nws/documents.rs` | Typed, lenient serde structs for the NWS documents used |
| `nws/cache.rs`, `nws/limiter.rs` | Cache freshness and the upstream request limiter |
| `normalize.rs` | NWS documents to response types |
| `convert.rs` | Units, rounding, wind text and distance |

## HTTP request path

Axum layers wrap earlier layers, so the request order is: request-ID sanitization, creation, and propagation; HTTP observation and logging; baseline response headers; compression; REST transport-error mapping; the 50-second timeout; origin verification; public REST CORS; the 16 KiB request-body limit; REST body consumption; then routing. Responses unwind in reverse order. The error mapper converts REST transport rejections into the shared typed error envelope, while MCP retains its own body and protocol handling.

`/healthz`, `/version`, and `/metrics` do not call NWS. The metrics endpoint uses bounded, fixed label sets. Request logs record the sanitized request ID, method, matched route, status, and latency without logging the raw URI or query.

## Weather paths

Every user-facing interface calls the same `Weather` service. The full report can degrade when an optional hourly, alert, station, or observation source fails. The focused hourly operation resolves the location, discovers the NWS grid, fetches only the hourly forecast, normalizes up to 24 periods, and explicitly reports that alerts were not checked. It does not depend on the daily forecast, stations, observations, or alert request.

The NWS client admits only links on its configured origin. Forecast documents and observations remain fresh for up to 120 seconds; grid discovery remains fresh for up to six hours. Incomplete grid discovery is usable for the safe links it contains but expires immediately, so a later lookup retries discovery.

## Maintain the model

Preview or validate with the repository commands:

```sh
likec4 start docs/architecture
just check-docs
```

Change the LikeC4 model when a component, trust boundary, request path, cache, or deployment relationship changes. Keep detailed limits and operational procedures in the source-linked guides instead of copying them into another diagram format. The former Mermaid and Excalidraw copies were removed because maintaining equivalent diagrams by hand allowed them to disagree.
