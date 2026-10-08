#!/usr/bin/env python3
"""Exercise running HTTP API and both MCP transports. --live opts into NWS calls."""

import argparse
import atexit
from contextlib import contextmanager
import gzip
import json
import os
from pathlib import Path
import re
import select
import socket
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.error
import urllib.request


ROOT = Path(__file__).resolve().parents[1]
STARTUP_LOG_LIMIT = 8 * 1024


class SmokeFailure(RuntimeError):
    """A smoke phase failed with a concise, actionable diagnostic."""


@contextmanager
def phase(name):
    try:
        yield
    except SmokeFailure:
        raise
    except Exception as error:
        detail = str(error) or type(error).__name__
        raise SmokeFailure(f"{name} phase failed: {detail}") from None


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8790")
    parser.add_argument("--live", action="store_true")
    parser.add_argument("--spawn", action="store_true")
    parser.add_argument(
        "--binary",
        type=Path,
        help="weather-bridge binary to spawn and use for MCP stdio and CLI checks",
    )
    parser.add_argument(
        "--http-only",
        action="store_true",
        help="Check HTTP and MCP HTTP without requiring a local binary",
    )
    args = parser.parse_args(argv)
    if args.http_only and args.spawn:
        parser.error("--http-only cannot be combined with --spawn")
    return args


def cargo_binary(explicit=None):
    if explicit:
        return explicit.expanduser().resolve()
    result = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        timeout=30,
    )
    if result.returncode:
        detail = result.stderr.strip() or result.stdout.strip() or "cargo metadata failed"
        raise SmokeFailure(f"binary resolution phase failed: {detail}")
    try:
        target = Path(json.loads(result.stdout)["target_directory"])
    except (KeyError, TypeError, json.JSONDecodeError) as error:
        raise SmokeFailure(f"binary resolution phase failed: invalid cargo metadata: {error}") from None
    return target / "debug" / "weather-bridge"


def log_tail(log):
    log.flush()
    size = log.seek(0, os.SEEK_END)
    log.seek(max(0, size - STARTUP_LOG_LIMIT))
    tail = log.read(STARTUP_LOG_LIMIT).decode(errors="replace").strip()
    if size > STARTUP_LOG_LIMIT:
        tail = "[startup log truncated to last 8192 bytes]\n" + tail
    return tail or "[no startup logs captured]"


class SpawnedServer:
    def __init__(self, binary, port):
        self.log = tempfile.TemporaryFile()
        try:
            self.process = subprocess.Popen(
                [str(binary), "serve", "--bind", f"127.0.0.1:{port}"],
                stdout=subprocess.DEVNULL,
                stderr=self.log,
            )
        except Exception:
            self.log.close()
            raise
        self.closed = False
        atexit.register(self.close)

    def wait_ready(self, url):
        for _ in range(100):
            try:
                urllib.request.urlopen(url + "/healthz", timeout=1).close()
                return
            except (urllib.error.URLError, TimeoutError, OSError):
                if self.process.poll() is not None:
                    raise SmokeFailure(
                        "startup phase failed: server exited during startup\n"
                        + log_tail(self.log)
                    )
                time.sleep(0.1)
        raise SmokeFailure(
            "startup phase failed: server startup timed out\n" + log_tail(self.log)
        )

    def close(self):
        if self.closed:
            return
        self.closed = True
        atexit.unregister(self.close)
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=8)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        self.log.close()


class Smoke:
    def __init__(self, args, binary):
        self.args = args
        self.binary = binary

    def request(self, path, body=None, headers=None):
        request = urllib.request.Request(
            self.args.url + path,
            data=json.dumps(body).encode() if body else None,
            headers=headers or {},
        )
        try:
            with urllib.request.urlopen(request, timeout=55) as response:
                return response.status, json.load(response)
        except urllib.error.HTTPError as error:
            return error.code, json.load(error)

    def raw(self, path, headers=None, body=None):
        request = urllib.request.Request(
            self.args.url + path, data=body, headers=headers or {}
        )
        with urllib.request.urlopen(request, timeout=10) as response:
            return response.headers, response.read()

    @staticmethod
    def baseline(headers, path):
        assert headers["X-Content-Type-Options"] == "nosniff", path
        assert headers["Referrer-Policy"] == "no-referrer", path
        assert headers["Strict-Transport-Security"] == "max-age=31536000", path
        csp = headers["Content-Security-Policy"]
        assert csp and "unsafe-eval" not in csp, path

    @staticmethod
    def rpc(method, params=None, id=1):
        return dict(
            jsonrpc="2.0",
            id=id,
            method=method,
            **({"params": params} if params is not None else {}),
        )

    @staticmethod
    def check_docs_links(page):
        links = [urllib.parse.urlsplit(link) for link in re.findall(r'href="([^"]+)"', page)]
        assert any(
            link.scheme == "https" and link.hostname and link.path.endswith("/rest.html")
            and not link.username and not link.password and not link.query and not link.fragment
            for link in links
        ), "developer page must link to an HTTPS REST reference"

    def check_http(self):
        assert self.request("/healthz")[1]["status"] == "ok"
        pages = [
            ("/", "/assets/weather.js"),
            ("/developer", "MCP tools"),
            ("/developer", "REST reference"),
            ("/assets/weather.js", "/v1/weather"),
            ("/assets/developer.js", "tools/call"),
        ]
        for path, marker in pages:
            with urllib.request.urlopen(self.args.url + path, timeout=5) as response:
                assert response.status == 200 and marker in response.read().decode(), path
                if path.endswith(".js"):
                    assert "javascript" in response.headers["Content-Type"]
        for path in ["/docs/rest", "/assets/reference.js", "/assets/scalar-1.72.1.js"]:
            try:
                urllib.request.urlopen(self.args.url + path, timeout=5)
                raise AssertionError(path + " should be gone")
            except urllib.error.HTTPError as error:
                assert error.code == 404, path

        for path in ["/", "/developer"]:
            headers, page = self.raw(path)
            self.baseline(headers, path)
            csp = headers["Content-Security-Policy"]
            assert "script-src 'self';" in csp and "frame-ancestors 'none'" in csp, (
                path,
                csp,
            )
            assert headers["Cache-Control"] == "public, max-age=300", path
            html = page.decode()
            if path == "/developer":
                self.check_docs_links(html)
            assert "<script>" not in html, path
            for source in re.findall(r'<script[^>]*\ssrc="([^"]+)"', html):
                assert source.startswith("/") and not source.startswith("//"), (path, source)

        headers, _ = self.raw("/v1/cities?q=Seattle")
        self.baseline(headers, "/v1/cities")
        assert headers["Content-Security-Policy"].startswith("default-src 'none'")
        assert headers["Access-Control-Allow-Origin"] == "*"
        assert headers["Cache-Control"] == "public, max-age=86400"
        assert self.raw("/")[0]["Access-Control-Allow-Origin"] is None
        assert self.request("/v1/cities?q=Seattle")[1]["data"][0]["name"] == "Seattle"
        assert self.request("/v1/weather?city=Springfield")[0] == 409
        assert self.request("/v1/weather?lat=999&lon=0")[0] == 400

    def mcp_setup(self):
        headers = {
            "Content-Type": "application/json",
            "Accept": "application/json, text/event-stream",
            "MCP-Protocol-Version": "2025-11-25",
        }
        init = self.rpc(
            "initialize",
            {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "weather-bridge-smoke", "version": "1"},
            },
        )
        return headers, init

    def check_mcp_http(self):
        headers, init = self.mcp_setup()
        status, body = self.request("/mcp", init, headers)
        assert status == 200, body
        assert body["result"]["capabilities"]["tools"] is not None
        response_headers, data = self.raw(
            "/mcp",
            {**headers, "Accept-Encoding": "gzip"},
            json.dumps(init).encode(),
        )
        self.baseline(response_headers, "/mcp")
        assert response_headers["Content-Encoding"] == "gzip"
        assert json.loads(gzip.decompress(data))["result"]["capabilities"]["tools"] is not None
        response_headers, data = self.raw(
            "/openapi.json", {"Accept-Encoding": "gzip"}
        )
        assert response_headers["Content-Encoding"] == "gzip"
        assert response_headers["Access-Control-Allow-Origin"] == "*"
        assert len(data) < len(gzip.decompress(data)) // 3
        assert json.loads(gzip.decompress(data))["openapi"].startswith("3.1")

        bad_origin = urllib.request.Request(
            self.args.url + "/mcp",
            data=json.dumps(init).encode(),
            headers={**headers, "Origin": "https://untrusted.example"},
        )
        try:
            urllib.request.urlopen(bad_origin, timeout=10).close()
            raise AssertionError("Untrusted MCP Origin was accepted")
        except urllib.error.HTTPError as error:
            assert error.code == 403, error.code

        status, body = self.request("/mcp", self.rpc("tools/list"), headers)
        assert status == 200, body
        assert {tool["name"] for tool in body["result"]["tools"]} == {
            "search_cities",
            "get_weather",
            "get_hourly_forecast",
            "get_active_alerts",
        }
        status, body = self.request(
            "/mcp",
            self.rpc(
                "tools/call",
                {"name": "search_cities", "arguments": {"query": "Seattle, WA"}},
            ),
            headers,
        )
        assert status == 200, body
        assert body["result"]["structuredContent"]["data"][0]["state"] == "WA"
        status, body = self.request(
            "/mcp",
            self.rpc(
                "tools/call",
                {"name": "get_weather", "arguments": {"city": "Springfield"}},
            ),
            headers,
        )
        assert status == 200, body
        assert body["result"]["isError"]

    def check_mcp_stdio(self):
        _, init = self.mcp_setup()
        process = subprocess.Popen(
            [str(self.binary), "mcp"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

        def send(message):
            process.stdin.write(json.dumps(message) + "\n")
            process.stdin.flush()

        def read(id):
            while True:
                if not select.select([process.stdout], [], [], 10)[0]:
                    raise RuntimeError("MCP response timed out")
                line = process.stdout.readline()
                if not line:
                    raise RuntimeError("MCP process exited before responding")
                message = json.loads(line)
                if message.get("id") == id:
                    return message

        try:
            send(init)
            assert "result" in read(1)
            send({"jsonrpc": "2.0", "method": "notifications/initialized"})
            send(self.rpc("tools/list", id=2))
            assert len(read(2)["result"]["tools"]) == 4
            send(
                self.rpc(
                    "tools/call",
                    {"name": "search_cities", "arguments": {"query": "Seattle, WA"}},
                    id=3,
                )
            )
            assert read(3)["result"]["structuredContent"]["data"][0]["state"] == "WA"
        finally:
            if process.poll() is None:
                process.terminate()
            try:
                process.communicate(timeout=8)
            except subprocess.TimeoutExpired:
                process.kill()
                process.communicate()

    def cli(self, *arguments):
        return subprocess.run(
            [str(self.binary), *arguments],
            capture_output=True,
            text=True,
            timeout=30,
            env={**os.environ, "RUST_LOG": "off"},
        )

    def check_cli(self):
        result = self.cli("cities", "Seattle, WA")
        assert result.returncode == 0
        assert "Seattle, WA" in result.stdout
        assert not result.stdout.lstrip().startswith("{")
        result = self.cli("cities", "Seattle, WA", "--json")
        assert json.loads(result.stdout)["data"][0]["name"] == "Seattle"
        result = self.cli("weather", "Franklin")
        assert result.returncode == 2
        assert not result.stdout
        assert "--city-id" in result.stderr
        assert "Franklin, TN (id " in result.stderr
        result = self.cli("hourly", "Franklin", "--json")
        body = json.loads(result.stdout)
        assert result.returncode == 2
        assert body["errors"][0]["code"] == "AMBIGUOUS_CITY"
        assert len(body["errors"][0]["choices"]) == 19
        assert self.cli("alerts", "--lat", "47.6").returncode == 2
        assert self.cli("weather", "Seattle", "--city-id", "1").returncode == 2

    @staticmethod
    def sources_ok(data, alerts=True):
        failed = [
            warning
            for warning in data["warnings"]
            if "unavailable" in warning or "could not be checked" in warning
        ]
        assert not failed, failed
        if alerts:
            assert data["alertsStatus"] == "checked", data["alertsStatus"]

    def check_live(self):
        headers, _ = self.mcp_setup()
        status, body = self.request("/v1/weather?city=Seattle%2C%20WA")
        assert status == 200, body
        data = body["data"]
        assert data["location"]["name"] == "Seattle, WA"
        assert data["forecast"]
        self.sources_ok(data)
        status, metric = self.request("/v1/weather?city=Seattle%2C%20WA&units=metric")
        assert status == 200, metric
        assert metric["data"]["forecast"][0]["temperature"]["unit"] == "°C"
        status, coordinates = self.request("/v1/weather?lat=47.6062&lon=-122.3321")
        assert status == 200, coordinates
        assert coordinates["data"]["location"]["precision"] == "coordinates"
        status, hourly = self.request("/v1/forecast/hourly?city=Seattle%2C%20WA")
        assert status == 200, hourly
        assert "hourly" in hourly["data"]
        self.sources_ok(hourly["data"], alerts=False)
        status, alerts = self.request("/v1/alerts?city=Seattle%2C%20WA")
        assert status == 200, alerts
        self.sources_ok(alerts["data"])
        status, body = self.request(
            "/mcp",
            self.rpc(
                "tools/call",
                {"name": "get_weather", "arguments": {"city": "Seattle, WA"}},
            ),
            headers,
        )
        assert status == 200 and not body["result"].get("isError"), body
        self.sources_ok(body["result"]["structuredContent"]["data"])


def main(argv=None):
    args = parse_args(argv)
    binary = None
    if args.spawn or not args.http_only:
        with phase("binary resolution"):
            binary = cargo_binary(args.binary)

    server = None
    try:
        if args.spawn:
            with socket.socket() as sock:
                sock.bind(("127.0.0.1", 0))
                port = sock.getsockname()[1]
            args.url = f"http://127.0.0.1:{port}"
            with phase("startup"):
                server = SpawnedServer(binary, port)
                server.wait_ready(args.url)

        smoke = Smoke(args, binary)
        with phase("HTTP"):
            smoke.check_http()
        with phase("MCP HTTP"):
            smoke.check_mcp_http()
        if not args.http_only:
            with phase("MCP stdio"):
                smoke.check_mcp_stdio()
            with phase("offline CLI"):
                smoke.check_cli()
        if args.live:
            with phase("live NWS"):
                smoke.check_live()
        coverage = "HTTP, MCP HTTP" if args.http_only else "HTTP, MCP HTTP, MCP stdio, offline CLI"
        if args.live:
            coverage += ", live NWS city/coordinates/units/forecast/alerts"
        print(coverage + ": passed")
        return 0
    finally:
        if server:
            server.close()


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except SmokeFailure as error:
        print(f"smoke: {error}", file=sys.stderr)
        raise SystemExit(1) from None
