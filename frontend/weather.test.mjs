import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { mountWeatherPage } from './weather-page.ts';

const html = await readFile(new URL('../web/index.html', import.meta.url), 'utf8');

// A small DOM surface keeps these behavior tests independent of a browser download.
class Element {
    hidden = false;
    textContent = '';
    className = '';
    value = '';
    children = [];
    listeners = new Map();
    append(...children) {
        this.children.push(...children);
    }
    replaceChildren(...children) {
        this.children = children;
    }
    setAttribute() {}
    addEventListener(type, fn) {
        this.listeners.set(type, fn);
    }
    trigger(type) {
        return this.listeners.get(type)?.({ preventDefault() {} });
    }
    querySelectorAll() {
        return this.children;
    }
    querySelector() {
        return this.children[0];
    }
}
class Input extends Element {}
class Select extends Element {}
class Button extends Element {
    disabled = false;
}
class Anchor extends Element {}

function page() {
    const nodes = new Map(
        [...html.matchAll(/id="([^"]+)"/g)].map(([, id]) => {
            const Type =
                id === 'city'
                    ? Input
                    : id === 'units'
                      ? Select
                      : id === 'go'
                        ? Button
                        : id === 'jsonLink'
                          ? Anchor
                          : Element;
            return [id, new Type()];
        }),
    );
    nodes.get('city').value = 'Seattle, WA';
    nodes.get('units').value = 'us';
    const pending = [];
    let timer;
    let now = Date.parse('2026-10-02T03:00:00Z'),
        nextTimer = 0;
    const timers = new Map(),
        listeners = new Map();
    class Clock extends Date {
        constructor(...args) {
            super(...(args.length ? args : [now]));
        }
        static now() {
            return now;
        }
    }
    const document = {
        hidden: false,
        addEventListener: (name, fn) => listeners.set(name, fn),
        getElementById: (id) => nodes.get(id),
        createElement: () => new Element(),
        createElementNS: () => new Element(),
    };
    mountWeatherPage({
        document,
        location: { origin: 'http://localhost' },
        HTMLInputElement: Input,
        HTMLSelectElement: Select,
        HTMLButtonElement: Button,
        HTMLAnchorElement: Anchor,
        AbortController,
        Request,
        Response,
        Headers,
        URLSearchParams,
        Intl,
        Date: Clock,
        setTimeout: (fn, delay) => {
            timer = fn;
            const id = ++nextTimer;
            timers.set(id, { fn, at: now + delay });
            return id;
        },
        clearTimeout: (id) => {
            timers.delete(id);
        },
        fetch: (request) =>
            new Promise((resolve, reject) => pending.push({ request, resolve, reject })),
    });
    return {
        nodes,
        pending,
        timer: () => timer?.(),
        advance: async (ms) => {
            now += ms;
            for (const [id, t] of timers)
                if (t.at <= now) {
                    timers.delete(id);
                    t.fn();
                }
            await tick();
        },
        visibility: async (hidden) => {
            document.hidden = hidden;
            listeners.get('visibilitychange')?.();
            await tick();
        },
    };
}
const tick = () => new Promise((resolve) => setImmediate(resolve));
const json = (body, status = 200) =>
    new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

function report(alertsStatus = 'unavailable') {
    return {
        assembledAt: '2026-10-02T03:00:00Z',
        cacheMaxAgeSeconds: 120,
        location: {
            name: 'Seattle, WA',
            timeZone: 'America/Los_Angeles',
            precision: 'city-center',
            latitude: 47.6,
            longitude: -122.3,
        },
        current: {
            temperature: { value: 18, unit: '°C' },
            condition: 'Cloudy',
            windSpeed: null,
            humidityPercent: null,
            stale: true,
            observedAt: '2026-10-01T00:00:00Z',
            station: 'KSEA',
            stationDistanceKm: 5,
        },
        forecast: [],
        hourly: [],
        summary: 'Forecast text',
        units: 'metric',
        sources: { forecast: { issuedAt: null } },
        alertsStatus,
        alerts: [],
        warnings: ['Alert check failed.'],
    };
}

test('weather rendering keeps stale observations and failed alerts distinct from no alerts', async () => {
    const { nodes, pending } = page();
    nodes.get('search').trigger('submit');
    pending[0].resolve(json({ data: report() }));
    await tick();
    assert.equal(nodes.get('weather').hidden, false);
    assert.match(nodes.get('observed').textContent, /^Older observation/);
    assert.equal(nodes.get('alertBadge').textContent, 'Alerts unavailable');
    assert.equal(nodes.get('proseUnits').hidden, false);
    nodes.get('search').trigger('submit');
    pending[1].resolve(json({ data: report('checked') }));
    await tick();
    assert.equal(nodes.get('alertBadge').textContent, 'No active alerts');
});

test('older weather completions cannot release the button or replace newer choices', async () => {
    const { nodes, pending } = page();
    nodes.get('search').trigger('submit');
    nodes.get('city').value = 'Springfield';
    nodes.get('search').trigger('submit');
    assert.equal(pending[0].request.signal.aborted, true);
    pending[0].resolve(json({ data: report() }));
    await tick();
    assert.equal(nodes.get('go').disabled, true);
    assert.equal(nodes.get('weather').hidden, true);
    pending[1].resolve(
        json(
            {
                errors: [
                    {
                        detail: 'Choose a city.',
                        choices: [
                            {
                                id: 1,
                                name: 'Springfield',
                                state: 'IL',
                                stateName: 'Illinois',
                                country: 'US',
                            },
                        ],
                    },
                ],
            },
            409,
        ),
    );
    await tick();
    assert.equal(nodes.get('go').disabled, false);
    assert.equal(nodes.get('status').textContent, 'Choose a city.');
    assert.equal(nodes.get('suggestions').hidden, false);
    nodes.get('suggestions').children[0].trigger('click');
    assert.equal(new URL(pending[2].request.url).searchParams.get('cityId'), '1');
});

test('submitting cancels suggestion search and ignores its later response', async () => {
    const { nodes, pending, timer } = page();
    nodes.get('city').trigger('input');
    void timer();
    nodes.get('search').trigger('submit');
    assert.equal(pending[0].request.signal.aborted, true);
    pending[0].resolve(json({ data: [{ name: 'Seattle', state: 'WA' }] }));
    await tick();
    assert.equal(nodes.get('suggestions').hidden, true);
});

test('gateway HTML shows the fallback, hides weather and releases the button', async () => {
    const { nodes, pending } = page();
    nodes.get('search').trigger('submit');
    pending[0].resolve(new Response('<html>Gateway unavailable</html>', { status: 503 }));
    await tick();
    assert.equal(nodes.get('status').textContent, 'Weather is temporarily unavailable. Try again.');
    assert.equal(nodes.get('status').className, 'status error');
    assert.equal(nodes.get('weather').hidden, true);
    assert.equal(nodes.get('go').disabled, false);
});

test('invalid successful JSON is a visible failure, never a weather report', async () => {
    const { nodes, pending } = page();
    nodes.get('search').trigger('submit');
    pending[0].resolve(new Response('<html>Invalid success</html>', { status: 200 }));
    await tick();
    assert.equal(nodes.get('status').textContent, 'Weather is temporarily unavailable. Try again.');
    assert.equal(nodes.get('weather').hidden, true);
    assert.equal(nodes.get('go').disabled, false);
});

async function startFresh() {
    const b = page();
    b.nodes.get('search').trigger('submit');
    b.pending[0].resolve(json({ data: report('checked') }));
    await tick();
    return b;
}
test('expires and repeatedly refreshes visible reports without hiding weather', async () => {
    const b = await startFresh();
    await b.advance(120000);
    assert.equal(b.pending.length, 2);
    assert.equal(b.nodes.get('alertBadge').textContent, 'Alerts need refresh');
    assert.equal(b.nodes.get('weather').hidden, false);
    b.pending[1].resolve(
        json({ data: { ...report('checked'), assembledAt: '2026-10-02T03:02:00Z' } }),
    );
    await tick();
    assert.equal(b.nodes.get('alertBadge').textContent, 'No active alerts');
    await b.advance(120000);
    assert.equal(b.pending.length, 3);
});
test('hidden tabs pause polling and refresh on return', async () => {
    const b = await startFresh();
    await b.visibility(true);
    await b.advance(120000);
    assert.equal(b.pending.length, 1);
    await b.visibility(false);
    assert.equal(b.pending.length, 2);
    assert.equal(b.nodes.get('alertBadge').textContent, 'Alerts need refresh');
});
test('failed refresh retains visibly stale weather and retries', async () => {
    const b = await startFresh();
    await b.advance(120000);
    b.pending[1].reject(Error('Network unavailable'));
    await tick();
    assert.equal(b.nodes.get('weather').hidden, false);
    assert.equal(b.nodes.get('alertBadge').textContent, 'Alerts need refresh');
    assert.match(b.nodes.get('status').textContent, /out of date/);
    await b.advance(120000);
    assert.equal(b.pending.length, 3);
});
test('CDN Age and short remaining TTL expire badges before the polling floor', async () => {
    const b = page();
    b.nodes.get('search').trigger('submit');
    b.pending[0].resolve(
        new Response(JSON.stringify({ data: report('checked') }), {
            headers: {
                'content-type': 'application/json',
                'cache-control': 'public, max-age=120',
                age: '119',
            },
        }),
    );
    await tick();
    await b.advance(1000);
    assert.equal(b.pending.length, 1);
    assert.equal(b.nodes.get('alertBadge').textContent, 'Alerts need refresh');
    await b.advance(9000);
    assert.equal(b.pending.length, 2);
});
test('timeout releases refresh request and schedules a retry', async () => {
    const b = await startFresh();
    await b.advance(120000);
    await b.advance(55000);
    assert.equal(b.pending[1].request.signal.aborted, true);
    const e = Error('Abort');
    e.name = 'AbortError';
    b.pending[1].reject(e);
    await tick();
    assert.match(b.nodes.get('status').textContent, /timed out/);
    assert.equal(b.nodes.get('go').disabled, false);
    await b.advance(120000);
    assert.equal(b.pending.length, 3);
});

test('observation staleness advances as the displayed report ages', async () => {
    const b = page();
    b.nodes.get('search').trigger('submit');
    const data = report('checked');
    data.current.stale = false;
    data.current.observedAt = '2026-10-02T01:01:00Z';
    b.pending[0].resolve(json({ data }));
    await tick();
    assert.doesNotMatch(b.nodes.get('observed').textContent, /Older observation/);
    await b.advance(120000);
    assert.match(b.nodes.get('observed').textContent, /Older observation/);
});

test('a degraded refresh displays unavailable alerts rather than an all-clear', async () => {
    const b = await startFresh();
    await b.advance(120000);
    b.pending[1].resolve(
        json({ data: { ...report('unavailable'), assembledAt: '2026-10-02T03:02:00Z' } }),
    );
    await tick();
    assert.equal(b.nodes.get('alertBadge').textContent, 'Alerts unavailable');
    assert.equal(b.nodes.get('weather').hidden, false);
    await b.advance(120000);
    assert.equal(b.nodes.get('alertBadge').textContent, 'Alerts need refresh');
});
