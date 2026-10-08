# Offline development fixtures

The development fixture runs the real Weather Bridge HTTP and MCP app against a
loopback NWS substitute. It makes no NWS requests and binds only to
`127.0.0.1:8790`.

Choose one explicit scenario:

```sh
cargo run --locked --example dev-fixture -- healthy
cargo run --locked --example dev-fixture -- stale
cargo run --locked --example dev-fixture -- alerts-unavailable
```

The page at <http://127.0.0.1:8790/> has a **SYNTHETIC OFFLINE FIXTURE** banner.
Every response also has
`X-Weather-Bridge-Fixture: synthetic-offline-data`. These labels distinguish
fixture values from live NWS weather. Stop the server with Ctrl-C; shutdown is
bounded to five seconds.

| Scenario | Observation | Alert check |
| --- | --- | --- |
| `healthy` | Fresh synthetic station observation | Succeeds with one synthetic alert |
| `stale` | Fixed 2020 timestamp, reported as stale | Succeeds with one synthetic alert |
| `alerts-unavailable` | Fresh synthetic station observation | Fails and reports `alertsStatus: "unavailable"` |

Try the REST behavior:

```sh
curl -i 'http://127.0.0.1:8790/v1/weather?city=Seattle%2C%20WA'
curl -i 'http://127.0.0.1:8790/v1/weather?city=Springfield'
curl -i 'http://127.0.0.1:8790/v1/cities?q=Springfield'
```

`Springfield` is deliberately ambiguous for weather lookup. The response is a
409 `AMBIGUOUS_CITY` error with choices; city search returns the choices without
calling the fixture NWS server.

The same process exposes Streamable HTTP MCP at
<http://127.0.0.1:8790/mcp>. For example, this calls the real `get_weather` MCP
tool over the synthetic service:

```sh
curl -i http://127.0.0.1:8790/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H 'MCP-Protocol-Version: 2025-11-25' \
  --data '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_weather","arguments":{"city":"Seattle, WA"}}}'
```

This fixture demonstrates deterministic service states for development and
tests. It does not prove that the live NWS service is reachable or that current
NWS documents still parse. Use `python3 scripts/smoke.py --spawn --live` for the
opt-in live check when changing NWS fetching or parsing.
