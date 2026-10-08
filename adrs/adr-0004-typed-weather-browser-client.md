# ADR-0004: Use generated types for the weather browser client

## Status

Accepted, 2026-10-01. Implements the downstream client direction in [ADR-0003](adr-0003-generate-openapi-from-rust.md).

Scope: the weather page's REST calls and rendering. The MCP explorer continues to use live MCP tool discovery.

## Context

The weather page currently builds REST query strings and reads response fields in handwritten JavaScript. Backend changes can pass Rust checks while leaving the browser with obsolete parameter names or response assumptions. Generating OpenAPI from Rust gives us an authoritative input for checking this consumer.

The page has two REST calls, city search and weather reports. It needs request cancellation, ambiguous-city choices, safe text rendering, and clear distinctions between observations, forecasts, stale data and failed alert checks. Gateway failures can be HTML or empty bodies rather than the service's JSON error envelope.

### Options evaluated

| Option | Fit and trade-off |
| --- | --- |
| **Generated TypeScript types with openapi-fetch** | Checks paths, query parameters, success/error envelopes and rendering against OpenAPI without generating a large SDK implementation. Adds a small fetch library and a developer build step. |
| **Full generated SDK** | Provides named operation functions, but adds generator configuration and generated implementation for only two calls. |
| **Handwritten JavaScript fetch calls** | Keeps development build-free, but cannot detect contract drift in consumers. |
| **Frontend framework migration** | Could add component conventions, but does not itself establish contract correctness and grows this task's scope. |

## Decision

Generate TypeScript types from the local OpenAPI artifact with **openapi-typescript**, use **openapi-fetch** for REST calls, and bundle the TypeScript weather page into the existing embedded JavaScript asset with **esbuild**.

Keep the page's DOM implementation and visual design. This decision introduces no UI framework or external browser dependency fetches.

## Rationale

openapi-typescript supports OpenAPI 3.1. openapi-fetch uses the generated path types to check requests and responses. Rendering can reference generated component types, making response-field changes visible to the TypeScript compiler. A single bundle preserves the existing asset route and same-origin content security policy.

Types provide compile-time checks, not runtime validation of every response. The client must handle transport failures and non-JSON error responses explicitly; it must not turn malformed success responses into fabricated weather.

## Implementation

- Pin dependencies and commit the npm lockfile. Use Node 24 in CI, with the supported development version documented in `package.json`.
- Commit generated declarations and the browser bundle. Ordinary Cargo builds and static documentation builds consume these artifacts without requiring Node.
- Generate from the repository's `openapi.json`, never from a live deployment. Update Rust OpenAPI first, then regenerate client types and the bundle.
- Enable strict TypeScript checks and `noUncheckedIndexedAccess`; derive wire types from generated declarations rather than duplicating interfaces or suppressing contract errors with broad casts.
- Keep requests on the page's own origin and pass cancellation signals through the client. Only the active request may update the page after a replacement request starts.
- Preserve structured service errors and ambiguous-city choices. Give HTML, empty and invalid JSON gateway failures a generic unavailable message.
- Retain safe DOM text rendering, weather/alert semantics and the current accessibility interactions.
- Add non-mutating generated-artifact checks, typechecking and client/UI regression tests to `just check` and CI. Generate into memory or temporary output so checks cannot silently repair stale artifacts.

Source linting and formatting use Oxc as recorded in [ADR-0005](adr-0005-frontend-oxc-quality-checks.md).

## Consequences

- Contract changes can now fail frontend typechecking as well as backend tests.
- Running the application and building the static docs remain independent of Node; frontend editing and full contributor checks require installed npm dependencies.
- The generated bundle is larger than the current handwritten script. The application still serves one browser asset without a CDN or runtime module loader.
- Generator and bundler upgrades can change checked-in artifacts; review those diffs and verify freshness checks on the supported CI runtime.
- Typechecking does not prove runtime availability or location-rule correctness. Existing backend tests and focused browser behavior tests remain necessary.

## Success criteria

- Regeneration is deterministic and stale types or bundles fail checks without modifying the checkout.
- The weather page uses typed client calls for both REST operations and rendering uses generated response types.
- Regression tests cover ambiguity, cancellation, replacement requests, non-JSON errors and stale/failed-alert presentation.
- The bundled page works with the existing server asset route and security policy.

## References

- [openapi-typescript: supported specifications and generation](https://openapi-ts.dev/introduction)
- [openapi-fetch: typed client and typechecking](https://openapi-ts.dev/openapi-fetch/)
- [esbuild: bundling and TypeScript](https://esbuild.github.io/getting-started/)
- [Weather Bridge API behavior](../README.md)
