# Weather Bridge

US weather by city name or coordinates, served as a web page, REST API, CLI and MCP server.

- Data comes from the public National Weather Service (NWS) API and a bundled GeoNames city index.
- One Rust service powers every interface, so they all return the same answers.
- No API key, database or paid service is needed. Node and pnpm are needed only for frontend development and contributor checks.

Try it: [weather page](https://bridge.wx.mrkd.co/), [developer page](https://bridge.wx.mrkd.co/developer), and [API docs](https://marknorgren.github.io/weather-bridge/).

## Run it

Requires Git and Rust 1.88 or later; `rust-toolchain.toml` pins the version CI uses.

```sh
git clone https://github.com/marknorgren/weather-bridge.git
cd weather-bridge
cargo run --locked -- serve
```

Open http://127.0.0.1:8790. Search for a city, pick a match, and switch between °F/mph and °C/km/h. The page shows station observations, the NWS plain-language outlook, hourly forecasts and active alerts. It loads nothing from other sites.

For an offline preview and source-edit workflow, see [Local development](docs/development.md). Synthetic fixtures include healthy, stale, and failed-alert states.

## Use the interfaces

```sh
cargo run --locked -- weather "Seattle, WA"
cargo run --locked -- weather "Seattle, WA" --units metric --json
cargo run --locked -- cities "Springfield"
curl 'http://127.0.0.1:8790/v1/weather?city=Seattle%2C%20WA'
```

The CLI also provides `hourly` and `alerts`. Pass a city name, `--city-id`, or both `--lat` and `--lon`.
REST accepts `city`, `cityId`, or both `lat` and `lon`. Ambiguous names return choices; the service never guesses.

Run `cargo build --locked`, then configure an MCP client to launch `target/debug/weather-bridge mcp` for stdio.
The HTTP server exposes Streamable HTTP at `/mcp` and a tool explorer at `/developer#mcp`.
Logs go to stderr; MCP stdio stdout carries protocol messages only.

Read the [interface and runtime reference](docs/reference.md) for commands, exit codes, schemas, configuration, caching, coverage, and limits.
The generated [OpenAPI contract](openapi.json) is also served at `/openapi.json`.

## Read weather correctly

- `current` is a station observation, separate from forecasts. Check `observedAt`, `ageSeconds`, and `stale`.
- Observations older than two hours are stale. Missing measurements stay null.
- An empty alert list with `alertsStatus: "checked"` means none are active. `unavailable` means the check failed, including malformed alert entries.
- The browser refreshes visible reports automatically and labels expired reports and alert checks when a refresh fails.
- Forecast and alert wording comes from NWS. Forecast text retains its original units; alert instructions are preserved.
- Coverage is the US and supported territories. City centers and nearby stations approximate a location, not a street address.

## Develop and operate

- [Local development](docs/development.md): prerequisites, setup, synthetic fixtures, and the source watcher.
- [Contributing](docs/contributing.md): changes through the shared service, generation, and verification, including `just check-browser` for offline weather/MCP browser fixtures.
- [Architecture](docs/architecture/README.md): code layout and the canonical LikeC4 model.
- [Operations](docs/operations.md): liveness, build identity, logs, request IDs, and process metrics.
- [Adapting the project](docs/adapting.md): identity, provider, and deployment changes for a fork.
- [Static docs site](docs/site/README.md): preview and publish the REST reference and MCP guide.
- [AWS deployment and usage guard](infra/aws/README.md): verified release artifacts, deployment, shutdown, and recovery.

The server's `/developer` page links to the REST reference and discovers MCP tools live.
Scalar 1.72.1 renders the separate Pages REST reference; the Rust binary does not include it.
See its [license and integrity notice](web/vendor/README.md).

Public hosting requires TLS, a contact User-Agent, trusted MCP hosts and origins, gateway rate limits, and resource-limit review.
There is no uptime guarantee. No database or radar ingestion is implemented.

## Support and contributions

This is a personal project. Bug reports and focused pull requests are welcome; use the [contributor guide](docs/contributing.md) for checks and workflow. Maintenance is best effort, with no guaranteed response time or uptime. Weather Bridge is not an emergency alert delivery service; consult official NWS sources for safety-critical decisions.

Report suspected vulnerabilities privately using the [security reporting policy](.github/SECURITY.md).

## Data and licenses

- Weather data comes from NWS. See its [API access and fair-use policies](https://www.weather.gov/documentation/services-web-api).
- City data comes from [GeoNames](https://www.geonames.org/) under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/). [data/SOURCE.md](data/SOURCE.md) documents the filtering.
- Code is MIT licensed. The MIT license does not replace the city-data license.
- No endorsement by NWS or GeoNames is implied.
