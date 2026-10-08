# ADR-0008: Use pnpm for the frontend toolchain

## Status

Accepted, 2026-10-05. Amends the package manager commands in [ADR-0004](adr-0004-typed-weather-browser-client.md), [ADR-0005](adr-0005-frontend-oxc-quality-checks.md) and [ADR-0007](adr-0007-browser-fixture-regressions.md). Where those records name `npm ci`, `npm run`, `npx` or the npm lockfile, use the pnpm equivalents below.

## Context

The frontend toolchain (openapi-fetch, openapi-typescript, esbuild, Oxlint, Oxfmt, TypeScript and Playwright) was installed with npm from `package-lock.json`. The maintainer's other repositories use pnpm, and the maintainer's machines disable dependency install scripts by default. npm runs every dependency install script unless a global setting stops it, so the scripts that ran in CI could differ from the scripts that ran locally.

## Decision

Use **pnpm 11** for frontend dependencies and scripts.

- Commit `pnpm-lock.yaml`. It was generated with `pnpm import` from the previous `package-lock.json`, so every package keeps the same pinned version. Do not edit the lockfile by hand; regenerate it from `package.json`.
- Pin the exact pnpm version with the `packageManager` field in `package.json`. pnpm and `pnpm/action-setup` read this field, so local and CI installs use the same version.
- Install with `pnpm install --frozen-lockfile` in `just setup` and CI. Run scripts with `pnpm run` and package binaries with `pnpm exec`.
- Record dependency build scripts in `pnpm-workspace.yaml` under `allowBuilds`. pnpm 11 stops an install when a build script is neither allowed nor denied. The esbuild `postinstall` script only validates and caches the platform binary that pnpm already installs as an optional dependency, so it is denied. Oxlint, Oxfmt and the type-aware companion ship native binaries as optional dependencies without build scripts.
- Keep pnpm's default isolated `node_modules` layout. esbuild writes the resolved path of each bundled module into `web/weather.js` as a comment, and the isolated layout resolves `openapi-fetch` to `node_modules/.pnpm/openapi-fetch@<version>/node_modules/openapi-fetch/...`. `frontend/build.mjs` sets esbuild's `preserveSymlinks: true`, which keeps the `node_modules/openapi-fetch/...` path, so generated files stay byte-identical.
- One-off tool installs outside the checkout (LikeC4 and the documentation Playwright package) keep their isolated `npm install --prefix ... --no-save --ignore-scripts` form. They have no project lockfile and do not use the frontend dependencies.

## Rationale

pnpm runs only the dependency build scripts that the repository approves, so local and CI installs execute the same reviewed code. A pinned `packageManager` version removes package manager drift between contributors and CI. The isolated layout also rejects imports of undeclared dependencies.

## Consequences

- Contributors need pnpm 11 or later. `just doctor setup` checks for it and gives the installation command.
- `scripts/doctor.py` reads the root importer of `pnpm-lock.yaml` to compare direct package versions with `node_modules`.
- The source watcher treats `pnpm-lock.yaml` and `pnpm-workspace.yaml` as build inputs.
- A new dependency with a build script stops `pnpm install` until a maintainer allows or denies it in `pnpm-workspace.yaml`.
- Generated file headers from `frontend/build.mjs` and its outputs name `pnpm run generate`; the generator and outputs were updated together.

## Success criteria

- `pnpm install --frozen-lockfile` completes without an interactive prompt, locally and in CI.
- `pnpm run check`, `just check`, `just check-browser` and `just doctor contributor` pass.
- The lockfile contains the same package versions as the previous `package-lock.json`.

## References

- [pnpm import](https://pnpm.io/cli/import)
- [pnpm settings](https://pnpm.io/settings)
- [pnpm/action-setup](https://github.com/pnpm/action-setup)
