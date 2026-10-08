# ADR-0005: Enforce frontend quality with Oxc

## Status

Accepted, 2026-10-01. Complements [ADR-0004](adr-0004-typed-weather-browser-client.md).

## Context

The typed weather client introduces handwritten TypeScript, generated declarations and a bundled JavaScript artifact alongside the existing MCP explorer. Typechecking catches contract mismatches, but does not cover all promise handling, unsafe type operations or formatting consistency. The frontend must finish contributor checks with zero lint errors and zero lint warnings.

### Options evaluated

| Option | Fit and trade-off |
| --- | --- |
| **Oxlint, type-aware linting and Oxfmt** | Provides Oxc's correctness checks, additional type-based checks and consistent formatting. Requires pinned native tooling and a compatible TypeScript version. |
| **ESLint and Prettier** | Established alternatives, but introduce a separate tooling stack where the requested Oxc tools provide the needed coverage. |
| **Typechecking alone** | Checks types and API consumption, but misses lint-specific correctness and maintainability issues. |

## Decision

Use **Oxlint with type-aware checks** and **Oxfmt** for handwritten frontend code, treat lint warnings as failures, and compose these with strict typechecking, artifact freshness checks and behavior tests in `npm run check`.

## Rationale

Oxc's correctness defaults provide a useful baseline. Type-aware checks add promise-handling and unsafe-operation rules that ordinary syntax linting cannot enforce. Formatter-owned style avoids duplicating style policy in lint rules. The frontend uses DOM APIs rather than JSX or a UI framework, so framework-specific plugins would not provide meaningful coverage.

Keep the TypeScript 5 compiler compatible with openapi-typescript's compiler-API dependency. Oxlint's type-aware companion supplies its own TypeScript 7 compiler semantics; verify that checks run against this project's configuration rather than forcing an incompatible generator peer upgrade.

## Implementation

- Pin Oxlint, its type-aware companion, Oxfmt and the compatible TypeScript compiler in the npm lockfile. Keep `tsc --noEmit` as an independent typecheck.
- Use explicit configuration for browser code, Node build/test scripts, correctness checks and selected promise/unsafe-operation rules. Make lint return a failure for any warning with `--deny-warnings`.
- Lint and format handwritten weather/client/build/test code and the existing MCP explorer. Exclude generated declarations, the generated browser bundle and vendored third-party assets from source formatting/linting; generated files remain protected by freshness checks and typechecking where applicable.
- Apply fixes to code rather than blanket disabling rules or introducing broad casts. Any future exception needs a narrow scope and an explanation.
- Provide separate lint, format-check and format-write commands. CI runs non-mutating checks; automatic fixes are an explicit developer action.
- Set four-space indentation explicitly in `.oxfmtrc.json` so inherited editor settings cannot make local formatting differ from CI.
- Run the same frontend command from `just check` and CI after `npm ci`.

## Consequences

- Lint errors and warnings block validation locally and in CI.
- Formatting changes to the MCP explorer are included without changing its discovery protocol or UI design.
- Native-tool and compiler compatibility must be reviewed during upgrades. Locked dependencies and reviewed configuration make the policy reproducible.
- Zero diagnostics means compliance with the configured rules, not proof of bug-free code or complete runtime validation. Regression and transport tests remain required.

## Success criteria

- `npm run check` reports zero lint errors and zero warnings, passes formatting and typechecking, and passes frontend behavior and artifact checks.
- Deliberately introducing a lint violation or formatting drift makes the relevant check fail.
- Contributor documentation identifies the enforced commands, source scope and generated-file exclusions.

## References

- [Oxlint: correctness defaults and integration](https://oxc.rs/docs/guide/usage/linter.html)
- [Oxlint type-aware linting](https://oxc.rs/docs/guide/usage/linter/type-aware.html)
- [Oxfmt](https://oxc.rs/docs/guide/usage/formatter.html)
