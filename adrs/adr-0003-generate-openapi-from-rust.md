# ADR-0003: Generate OpenAPI from the Rust API

## Status

Accepted, 2026-10-01.

Scope: REST contract generation and its relationship to MCP schemas and future clients.

## Context

Before this decision, Weather Bridge maintained `openapi.json` by hand. The server embedded that file, and the documentation build copied it with a deployment-specific server URL. Contract tests compared fixture responses and Rust type shapes with the document, but contributors still needed to update both Rust code and JSON.

Axum routes, query types, response envelopes and service errors already lived in Rust. Response types derived `schemars::JsonSchema` for MCP. REST handlers returned Axum's erased `Response`, which hid their payload types from automatic operation inference.

### Requirements

- Make Rust routes and wire types authoritative for the REST contract.
- Reuse the schema definitions used by MCP; keep REST, CLI and MCP behavior in the shared `Weather` service.
- Preserve OpenAPI 3.1, existing operation IDs, query names, response envelopes, error codes, attribution and descriptions.
- Preserve nullable values, serialized required fields, closed response schemas, and the distinction between failed alert checks and no alerts.
- Keep input limits, location validation, NWS fetching, cache behavior, security headers and transport behavior intact.
- Produce a deterministic, reviewable artifact without starting a server or accessing NWS.
- Keep documentation builds able to consume a checked-in specification without requiring Rust or network access.

### Options evaluated

| Option | Fit and trade-off |
| --- | --- |
| **Aide with Axum and Schemars** | Generates OpenAPI from documented route registration and Rust types. Reuses the schema system already used by MCP. Requires typed response documentation and explicit metadata for behavior that types cannot express. |
| **Utoipa with utoipa-axum** | Registers handlers and collects OpenAPI through `OpenApiRouter`; uses `ToSchema` derives and handler annotations. Adds a second schema system alongside MCP's Schemars definitions. |
| **Specification first, generating clients or server scaffolding** | Useful when an independently designed contract owns the API. Generating only a client would leave this server and the handwritten specification able to drift; generating server scaffolding would require adapting an existing implementation. |
| **Handwritten specification with contract tests** | Retains the current workflow and dependencies, but still requires duplicate maintenance. |

## Decision

Generate OpenAPI 3.1 from the Rust API using **Aide**, sharing Schemars wire-type definitions with MCP, and generate any future clients downstream from that document.

The dependency direction is:

```text
Rust route registration + request/response types + operation metadata
                              ↓
                         openapi.json
                              ↓
                    documentation and clients
```

The checked-in `openapi.json` becomes a generated artifact. It is never edited directly. The server serves the generated contract; it does not depend on the checked-in artifact to define the contract.

## Rationale

Aide uses `schemars::JsonSchema` and wraps Axum routing with `ApiRouter`. This lets REST and MCP share schema ownership and places operation metadata next to the route registration that actually serves requests. Utoipa is a viable alternative, but maintaining its schema derives alongside Schemars would add another source of disagreement.

The implementation pins Aide `=0.16.0-alpha.4`: stable Aide 0.15 uses Schemars 0.9, while the existing MCP dependency uses Schemars 1. The prerelease supports Axum 0.8 and Schemars 1, allowing one schema system. This accepts prerelease integration risk; upgrades require the contract, runtime, MCP and dependency checks to pass and generated diffs to be reviewed. Reassess the pin when a compatible stable release is available.

Generation does not describe all runtime behavior. Location combinations, status mappings, caching headers, examples and explanatory text still need explicit documentation and behavioral checks. Those declarations belong beside the implementation, rather than in a separately maintained JSON document.

## Implementation

1. Verify and pin compatible Aide, Axum and Schemars dependencies against the project's supported Rust version and dependency policy.
2. Register public REST operations through Aide's documented router. Keep pages, assets and MCP transport outside the REST contract.
3. Expose concrete success envelopes and error bodies through typed response wrappers or operation trait implementations. Preserve existing status and cache-header rendering. Derive documented service error statuses from the existing code-to-status mapping where applicable.
4. Generate schemas from the same wire types as MCP, using serialization semantics for responses and deserialization semantics for inputs. Review custom schemas, nullable fields, required fields, enums and component references against the existing contract.
5. Use one contract-building function for the runtime endpoint and an offline export command. Serialize the runtime document once and retain the current endpoint headers and compression behavior.
6. Add `just generate-openapi` and a non-mutating freshness check to `just check` and CI. A stale checked-in document fails validation with instructions to regenerate it. The documentation builder continues to change only the server URL in its output copy.
7. Keep fixture response validation, error coverage and MCP structured-result checks. Replace redundant shape comparisons only when generation provides equivalent coverage. Complete `just check`; live smoke testing is required if implementation changes NWS fetching or parsing.

Frontend client adoption is recorded in [ADR-0004](adr-0004-typed-weather-browser-client.md). It follows contract generation and preserves request cancellation, ambiguous-city choices, non-JSON gateway error handling and weather semantics. The MCP explorer continues to discover tools through MCP rather than through REST OpenAPI.

## Consequences

- Rust field and enum changes flow into REST and MCP schemas without editing JSON.
- Generated contract changes remain visible in code review, and CI detects stale artifacts.
- Documentation builds retain their existing offline input.
- Aide adds a dependency and route/response integration work. Dependency and generator upgrades can change the artifact and require review.
- Shared schema ownership reduces duplication, but REST operations and MCP tools still have different transport metadata.
- Generated schemas and clients do not validate live NWS availability or prove runtime responses are correct; behavioral tests remain necessary.

## Success criteria

- Exporting twice produces identical output, and the runtime document matches the exported contract.
- Changing a wire field or documented route changes the generated artifact and fails the freshness check until regeneration.
- Existing fixture, error, MCP, header and smoke checks pass without changing public behavior.
- The documentation build consumes the generated artifact without starting the API or fetching dependencies.

## References

- [Aide: type-based generation and feature flags](https://docs.rs/aide/latest/aide/)
- [Aide's Axum integration](https://docs.rs/aide/latest/aide/axum/index.html)
- [Pinned Aide release and dependencies](https://docs.rs/crate/aide/0.16.0-alpha.4)
- [Utoipa's Axum router integration](https://docs.rs/utoipa-axum/latest/utoipa_axum/)
- [API behavior and contributor checks](../README.md)
- [Current contract tests](../tests/contract.rs)
- [Static documentation build](../scripts/build-docs.py)
