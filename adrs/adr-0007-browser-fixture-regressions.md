# Browser fixture regressions

Status: accepted.

Source: weather freshness review, the interrupted browser E2E task, and [Playwright Docker guidance](https://playwright.dev/docs/docker).
LLM involvement: implemented and documented by Codex using GPT-6.

## Decision

Use pinned Playwright 1.58.2 for the authored weather page and MCP explorer, with desktop and mobile viewports in Chromium and WebKit. Serve the actual checked-in HTML and JavaScript from an owned loopback-only server. Intercept API/MCP requests with explicit fixtures and abort every external request. Keep production, NWS, credentials and the cost guard outside this gate.

The suite covers fresh, stale and unavailable data; HTTP Age expiry; outage retention and recovery; request timeout and overlap; keyboard city selection and units; MCP discovery, structured results, invalid JSON arguments, tool errors and discovery recovery. Playwright screenshots cover both pages in every project. Store failure traces and an HTML report under target/browser-test. Capture screenshots directly in Playwright rather than maintaining a second shot-scraper wrapper around the same browser session.

## Boundaries

This is browser integration coverage of the shipped frontend, not proof of the Rust API, NWS availability or deployed infrastructure. Rust fixture and transport tests, offline smoke tests and the explicitly opted-in live NWS check cover those separate boundaries. The repository also has documentation-browser checks and a Rust demo fixture; these tests supplement them without replacing or importing unrelated changes. This decision supplements the generated-client and frontend-quality decisions without replacing documentation-browser coverage.

No browser target rejected by access policy is retried. The test server is a new owned fixture origin; no error-page or production navigation is involved. Linux cached official browser binaries provide local evidence; macOS native browsers and the pinned Rust compiler remain separate validation routes.

## Running and reviewing

Run npm ci, then npx playwright install --with-deps chromium webkit where browser installation is appropriate, then just check-browser. An existing matching official Playwright image can supply browsers without downloading them again. The test runner version must match the image version. CI uses the same pinned npm dependency, installs both browsers, runs fixtures and uploads reports even on failure. No retry masks a failing test.

Screenshots are review artifacts, with viewport overflow assertions; there are no platform-sensitive pixel goldens. The small fixture server intentionally has no proxy or API fallback. Unexpected fixture requests fail instead of reaching a real service.
