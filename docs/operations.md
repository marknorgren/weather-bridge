# Operations

Weather Bridge exposes three unauthenticated operator endpoints. They do not contact NWS:

- `GET /healthz` is process liveness only. It stays healthy when NWS is unavailable.
- `GET /version` returns the package version and embedded source revision as JSON.
- `GET /metrics` returns Prometheus text metrics. All labels come from fixed sets; caller
  locations, coordinates, paths, queries, headers, and other input never become labels.

All three responses use `Cache-Control: no-store`. Public deployments should let their
monitoring system reach `/metrics`; the endpoint contains only aggregate process counters,
fixed-bucket latency histograms, and build identity.

Every HTTP response carries `X-Request-ID`, including routing and middleware errors. A caller
may supply a correlation ID containing 1–128 ASCII letters, digits, `-`, `_`, or `.`. Other
values are replaced. Request logs contain that ID, the HTTP method, the matched route, status,
and latency. They never contain the raw URI or query string.

The ID correlates a request that reaches the Weather Bridge process. Shared caches may reuse a
cached response and its `X-Request-ID`; a cache hit does not reach the process and therefore does
not create an application log or increment these process metrics. Cache keys do not vary by the
request ID.

Local processes write compact, readable logs to stderr. The AWS Lambda environment is detected
from `AWS_LAMBDA_FUNCTION_NAME` and writes newline-delimited JSON to stderr. MCP stdio therefore
keeps stdout exclusively for protocol messages. `RUST_LOG` controls filtering in both modes;
the default is `weather_bridge=info,tower_http=warn`.

Release builds must set `WEATHER_BRIDGE_BUILD_REVISION` while compiling. Its value must be the
exact 40-character lowercase hexadecimal source commit. An ordinary local build embeds the
explicit value `unknown`. The value is compiled into the binary and appears in `--version`,
`/version`, the startup log, and `weather_bridge_build_info`; the library never reads it from
the runtime environment.

The bounded metrics are:

| Metric | Meaning |
| --- | --- |
| `weather_bridge_build_info` | Package version and embedded revision. |
| `weather_bridge_http_requests_total` | Responses by matched route and status class. |
| `weather_bridge_http_request_duration_seconds` | Response latency by matched route. |
| `weather_bridge_upstream_requests_total` | NWS request outcome by fixed endpoint kind. |
| `weather_bridge_upstream_request_duration_seconds` | NWS request latency by endpoint kind. |
| `weather_bridge_upstream_failures_total` | Transport, status, size, decoding, and required-field failures. |
| `weather_bridge_cache_access_total` | Document and grid-point cache hits and misses. |
| `weather_bridge_partial_reports_total` | Successful weather, hourly, or alerts output missing an optional source. |
| `weather_bridge_busy_total` | Lookups refused as `BUSY`: by the eight-lookup admission limit, or because the upstream request pace could not start a required request in time. |

Counters and histograms are process-local and reset when the process restarts. Replicas do not
share them.
