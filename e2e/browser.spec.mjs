import { test, expect } from '@playwright/test';

const now = '2026-10-02T03:00:00Z';
const city = { id: 5809844, name: 'Seattle', state: 'WA', stateName: 'Washington', country: 'US' };
function report(overrides = {}) {
    return {
        assembledAt: now,
        cacheMaxAgeSeconds: 120,
        units: 'us',
        location: {
            name: 'Seattle, WA',
            timeZone: 'America/Los_Angeles',
            precision: 'city-center',
            latitude: 47.6,
            longitude: -122.3,
        },
        current: {
            temperature: { value: 65, unit: '°F' },
            condition: 'Cloudy',
            windSpeed: null,
            humidityPercent: null,
            stale: false,
            observedAt: now,
            station: 'KSEA',
            stationDistanceKm: 5,
        },
        forecast: [],
        hourly: [],
        summary: 'Fixture forecast',
        sources: { forecast: { issuedAt: now } },
        alertsStatus: 'checked',
        alerts: [],
        warnings: [],
        ...overrides,
    };
}
function period(index) {
    return {
        name: `Day ${index + 1}`,
        isDaytime: true,
        startsAt: new Date(Date.parse(now) + index * 3600000).toISOString(),
        temperature: { value: 65 + index, unit: '°F' },
        condition: 'Cloudy',
        wind: '5 mph',
        precipitationProbabilityPercent: 20,
    };
}

const reply = (route, data, headers = {}) =>
    route.fulfill({ json: { data }, headers: { 'Cache-Control': 'max-age=120', ...headers } });
test.beforeEach(async ({ page }) => {
    await page.clock.install({ time: new Date(now) });
    await page.clock.pauseAt(new Date(now));
    await page.route('**/*', (route) =>
        new URL(route.request().url()).origin === 'http://127.0.0.1:4179'
            ? route.continue()
            : route.abort(),
    );
});
async function load(page) {
    await page.goto('/');
    await page.locator('#city').fill('Seattle, WA');
    await page.locator('#search').evaluate((form) => form.requestSubmit());
    await expect(page.locator('#weather')).toBeVisible();
}
async function capture(page, info, name) {
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(
        true,
    );
    const path = info.outputPath(`${name}.png`);
    await page.screenshot({ path, fullPage: true, animations: 'disabled' });
    await info.attach(name, { path, contentType: 'image/png' });
}

test('weather rendering, stale observations, alert status, and responsive screenshot', async ({
    page,
}, info) => {
    const errors = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.route('**/v1/weather?**', (route) =>
        reply(
            route,
            report({
                current: { ...report().current, stale: true },
                alertsStatus: 'unavailable',
                forecast: Array.from({ length: 4 }, (_, index) => period(index)),
                hourly: Array.from({ length: 24 }, (_, index) => period(index)),
            }),
        ),
    );
    await load(page);
    await expect(page.locator('#observed')).toContainText('Older observation');
    await expect(page.locator('#alertBadge')).toHaveText('Alerts unavailable');
    await expect(page.locator('.day')).toHaveCount(4);
    await expect(page.locator('.hour')).toHaveCount(8);
    await capture(page, info, 'weather');
    expect(errors).toEqual([]);
});

test('HTTP Age expires alerts, retains weather on outage, then recovers', async ({ page }) => {
    let calls = 0;
    await page.route('**/v1/weather?**', async (route) => {
        calls++;
        if (calls === 2)
            return route.fulfill({ status: 503, json: { errors: [{ detail: 'Fixture outage' }] } });
        return reply(
            route,
            report({ assembledAt: calls === 1 ? now : '2026-10-02T03:02:10Z' }),
            calls === 1 ? { Age: '119' } : {},
        );
    });
    await load(page);
    await expect(page.locator('#alertBadge')).toHaveText('No active alerts');
    await page.clock.runFor(1000);
    await expect(page.locator('#alertBadge')).toHaveText('Alerts need refresh');
    await page.clock.runFor(9000);
    await expect(page.locator('#status')).toContainText('Displayed weather may be out of date');
    await expect(page.locator('#weather')).toBeVisible();
    await page.clock.runFor(120000);
    await expect(page.locator('#alertBadge')).toHaveText('No active alerts');
    expect(calls).toBe(3);
});

test('keyboard city choices resolve ambiguity and metric selection', async ({ page }) => {
    await page.route('**/v1/cities?**', (route) => reply(route, [city]));
    await page.route('**/v1/weather?**', (route) => {
        const query = new URL(route.request().url()).searchParams;
        expect(query.get('cityId')).toBe(String(city.id));
        return reply(route, report({ units: query.get('units') }));
    });
    await page.goto('/');
    await page.locator('#city').fill('Seattle');
    await page.clock.runFor(250);
    await expect(page.locator('#suggestions')).toBeVisible();
    await page.locator('#city').press('ArrowDown');
    await page.keyboard.press('Enter');
    await expect(page.locator('#weather')).toBeVisible();
    await page.locator('#units').selectOption('metric');
    await expect(page.locator('#proseUnits')).toBeVisible();
});

test('overlapping canceled requests preserve the newest result', async ({ page }) => {
    let handled;
    const oldHandled = new Promise((resolve) => {
        handled = resolve;
    });
    let release;
    const held = new Promise((resolve) => {
        release = resolve;
    });
    let firstSeen;
    const first = new Promise((resolve) => {
        firstSeen = resolve;
    });
    let calls = 0;
    await page.route('**/v1/weather?**', async (route) => {
        calls++;
        if (calls === 1) {
            firstSeen();
            await held;
            await reply(route, report({ summary: 'Old result' })).catch(() => {});
            handled();
        } else await reply(route, report({ summary: 'Newest result' }));
    });
    await page.goto('/');
    await page.locator('#search').evaluate((form) => form.requestSubmit());
    await first;
    await page.locator('#city').fill('Seattle, WA');
    await page.locator('#search').evaluate((form) => form.requestSubmit());
    await expect(page.locator('#summary')).toHaveText('Newest result');
    release();
    await oldHandled;
    await expect(page.locator('#go')).toBeEnabled();
    await expect(page.locator('#summary')).toHaveText('Newest result');
});

test('request timeout releases controls and malformed JSON has a safe error', async ({ page }) => {
    let seen;
    const requested = new Promise((resolve) => {
        seen = resolve;
    });
    await page.route('**/v1/weather?**', () => {
        seen();
    });
    await page.goto('/');
    await page.locator('#search').evaluate((form) => form.requestSubmit());
    await requested;
    await page.clock.runFor(55000);
    await expect(page.locator('#status')).toContainText('timed out');
    await expect(page.locator('#go')).toBeEnabled();
    await page.unroute('**/v1/weather?**');
    await page.route('**/v1/weather?**', (route) =>
        route.fulfill({ contentType: 'application/json', body: '{private parser text' }),
    );
    await page.locator('#search').evaluate((form) => form.requestSubmit());
    await expect(page.locator('#status')).toHaveText(
        'Weather is temporarily unavailable. Try again.',
    );
});

const tool = {
    name: 'search_cities',
    description: 'Find a fixture city',
    annotations: { readOnlyHint: true },
    inputSchema: { type: 'object', properties: { query: { type: 'string' } }, required: ['query'] },
};
async function mcp(page, { outage = false, toolError = false } = {}, navigate = true) {
    await page.route('**/mcp', async (route) => {
        const request = route.request().postDataJSON();
        if (outage) return route.fulfill({ status: 503 });
        if (request.method === 'notifications/initialized')
            return route.fulfill({ status: 202, body: '' });
        const result =
            request.method === 'initialize'
                ? {
                      protocolVersion: '2025-11-25',
                      capabilities: {},
                      serverInfo: { name: 'fixture', version: '1' },
                  }
                : request.method === 'tools/list'
                  ? { tools: [tool] }
                  : { structuredContent: { cities: [city] }, isError: toolError };
        await route.fulfill({ json: { jsonrpc: '2.0', id: request.id, result } });
    });
    if (navigate) await page.goto('/developer#mcp');
}

test('MCP discovery, structured call, keyboard tabs, and responsive screenshot', async ({
    page,
}, info) => {
    await mcp(page);
    await expect(page.locator('#connection')).toContainText('1 tools discovered');
    await page.locator('#argument-query').fill('Seattle');
    await page.locator('#run-tool').click();
    await expect(page.locator('#call-status')).toHaveText('Tool completed.');
    await expect(page.locator('#result')).toContainText('Seattle');
    await capture(page, info, 'mcp');
    await page.locator('#mcp-tab').press('Home');
    await expect(page.locator('#rest-panel')).toBeVisible();
    await page.locator('#rest-tab').press('End');
    await expect(page.locator('#mcp-panel')).toBeVisible();
});

test('MCP discovery outage can be retried by switching tabs', async ({ page }) => {
    await mcp(page, { outage: true });
    await expect(page.locator('#connection')).toContainText('HTTP 503');
    await page.unroute('**/mcp');
    await mcp(page, {}, false);
    await page.locator('#rest-tab').click();
    await page.locator('#mcp-tab').click();
    await expect(page.locator('#connection')).toContainText('tools discovered');
    await expect(page.locator('#connection')).not.toHaveClass(/error/);
});

test('MCP rejects non-object JSON locally and distinguishes tool errors', async ({ page }) => {
    await mcp(page, { toolError: true });
    await expect(page.locator('#run-tool')).toBeVisible();
    await page.getByText('Use JSON arguments instead', { exact: true }).click();
    await page.locator('#use-json').check();
    await page.locator('#arguments').fill('[]');
    await page.locator('#run-tool').click();
    await expect(page.locator('#call-status')).toHaveText('Arguments must be a JSON object.');
    await page.locator('#arguments').fill('{"query":"Seattle"}');
    await page.locator('#run-tool').click();
    await expect(page.locator('#call-status')).toContainText('Tool returned an error');
    await expect(page.locator('#run-tool')).toBeEnabled();
});

test('ambiguous names return choices and original alert text is rendered safely', async ({
    page,
}) => {
    let calls = 0;
    await page.route('**/v1/weather?**', async (route) => {
        if (++calls === 1)
            return route.fulfill({
                status: 409,
                json: { errors: [{ detail: 'Choose a matching city', choices: [city] }] },
            });
        expect(new URL(route.request().url()).searchParams.get('cityId')).toBe(String(city.id));
        await reply(
            route,
            report({
                alerts: [
                    {
                        headline: 'Fixture alert',
                        event: 'Fixture warning',
                        description: '<script>window.fixtureInjected = true</script>',
                        instruction: 'Follow official instructions.',
                    },
                ],
            }),
        );
    });
    await page.goto('/');
    await page.locator('#search').evaluate((form) => form.requestSubmit());
    await expect(page.locator('#suggestions')).toBeVisible();
    await page.locator('#suggestions button').click();
    await expect(page.locator('#alertBadge')).toHaveText('1 active alert');
    await page.getByText('Fixture alert', { exact: true }).click();
    await expect(page.locator('#alerts')).toContainText('Follow official instructions.');
    await expect(page.locator('#alerts script')).toHaveCount(0);
    expect(await page.evaluate(() => window.fixtureInjected)).toBeUndefined();
});
