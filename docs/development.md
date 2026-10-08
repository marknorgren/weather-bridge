# Local development

Weather Bridge has two local server modes. `just dev` runs the normal service and uses live NWS data. `just dev-watch` runs synthetic fixture data entirely on loopback, rebuilds generated artifacts and Rust code after source edits, and restarts the fixture after a successful build.

## Check prerequisites

The prerequisite checker only inspects local commands and files. It does not install software, read credentials or contact cloud services.

```sh
just doctor run
just doctor setup
just doctor contributor
just doctor docs
just doctor release
just doctor all
```

Plain `just doctor` checks the runtime toolchain, Node, pnpm, and Just. It also compares direct frontend package versions with the lockfile and checks their command links. Docs and release tools remain opt-in.

The scopes are independent:

| Scope | Checks |
| --- | --- |
| `run` | Cargo and the pinned Rust compiler |
| `setup` | The `run` checks, Node 22.18+, pnpm 11+ and Just; does not require `node_modules` |
| `contributor` | The `run` checks, Node 22.18+, pnpm 11+, Just and direct frontend package versions and command links |
| `docs` | Node, LikeC4 1.59.4 and the pinned Playwright browser installation |
| `release` | cargo-lambda, Zig, Git, GitHub CLI, AWS CLI and Terraform executables |

Every missing prerequisite prints the exact command or official installation page needed to repair it. Cloud credentials are outside the check.

`just setup` prepares the contributor workflow by installing the versions locked in `pnpm-lock.yaml` and checking the required tools:

```sh
just setup
```

That command first checks the Rust toolchain, Node 22.18 or later, pnpm 11 or later and Just. It stops before installation when any of them are missing. It then runs `pnpm install --frozen-lockfile` in the repository and checks the complete contributor prerequisites, including direct package versions and command links. It does not install global packages. The `packageManager` field in `package.json` pins the pnpm version, and pnpm selects that version automatically. `pnpm-workspace.yaml` records which dependency build scripts may run, so installation does not stop for an approval prompt, and uses pnpm's default isolated `node_modules` layout. The frontend build enables `preserveSymlinks` so generated bundles retain their dependency paths. [ADR-0008](../adrs/adr-0008-pnpm-package-manager.md) records the package manager decision. Running the Rust service alone still needs only the `run` prerequisites and `cargo run --locked -- serve`.

## Offline edit loop

Start the default healthy scenario:

```sh
just dev-watch
```

Choose another explicit state when working on stale observations or alert failures:

```sh
just dev-watch stale
just dev-watch alerts-unavailable
```

The fixture serves http://127.0.0.1:8790, labels every response as synthetic offline data and never calls NWS. Development responses use `Cache-Control: no-store`, so a browser refresh sees the restarted binary rather than a cached page, script or API response.

For each handwritten source change, the watcher runs these steps in order:

1. Export `openapi.json` from the Rust route and wire types.
2. Generate `frontend/schema.d.ts` and bundle `web/weather.js`.
3. Build the `dev-fixture` example.
4. Stop the previous fixture process group and start the newly built binary.

The watcher tracks Rust, examples, shared fixtures, frontend source, HTML, city data, and package/config changes. It excludes generated files, `target/`, `node_modules/`, and Pages-only vendor assets, so generation does not cause another build.

If a generation or build command fails, the existing successfully built fixture stays running. The watcher prints the failed phase and waits. Fixing and saving a source file runs the pipeline again; a successful retry prints `build recovered` before replacing the server. Ctrl-C or termination stops the complete fixture process group, with a five-second bound before a forced stop.

The watcher asks Cargo metadata for its target directory. A custom target works without additional flags:

```sh
CARGO_TARGET_DIR=/tmp/weather-bridge-target just dev-watch
```

For a single fixture run without watching, use `just demo healthy`. The [fixture guide](development-fixtures.md) defines each scenario and shows REST/MCP examples.

To exercise live data, use `just dev`; live NWS verification remains an explicit `just check-live` operation.
