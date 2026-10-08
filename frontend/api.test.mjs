import assert from 'node:assert/strict';
import test from 'node:test';
import { createWeatherClient, LatestRequest } from './api.ts';

const json = (body, status = 200) =>
    new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

test('generated client serializes city IDs and units and forwards cancellation', async () => {
    const controller = new AbortController();
    const api = createWeatherClient('http://localhost', async (request) => {
        assert.equal(new URL(request.url).pathname, '/v1/weather');
        assert.equal(new URL(request.url).searchParams.get('cityId'), '5809844');
        assert.equal(new URL(request.url).searchParams.get('units'), 'metric');
        assert.equal(request.signal.aborted, false);
        controller.abort();
        assert.equal(request.signal.aborted, true);
        return json({ data: { location: { name: 'Seattle' } } });
    });
    const result = await api.GET('/v1/weather', {
        params: { query: { cityId: 5809844, units: 'metric' } },
        signal: controller.signal,
    });
    assert.equal(result.data.data.location.name, 'Seattle');
});

test('ambiguous cities retain actionable error detail and choices', async () => {
    const choices = [{ id: 1, name: 'Springfield', state: 'IL' }];
    const api = createWeatherClient('http://localhost', async () =>
        json(
            {
                errors: [
                    { status: '409', code: 'AMBIGUOUS_CITY', detail: 'Choose a city.', choices },
                ],
            },
            409,
        ),
    );
    const result = await api.GET('/v1/weather', { params: { query: { city: 'Springfield' } } });
    assert.equal(result.error.errors[0].detail, 'Choose a city.');
    assert.deepEqual(result.error.errors[0].choices, choices);
});

for (const [name, body, contentType] of [
    ['HTML', '<html>gateway</html>', 'text/html'],
    ['empty', '', 'application/json'],
    ['invalid JSON', '{', 'application/json'],
]) {
    test(`${name} gateway failures retain HTTP failure and a safe generic message`, async () => {
        const api = createWeatherClient(
            'http://localhost',
            async () =>
                new Response(body, { status: 502, headers: { 'content-type': contentType } }),
        );
        await assert.rejects(
            api.GET('/v1/weather', { params: { query: { city: 'Seattle, WA' } } }),
            {
                name: 'GatewayError',
                status: 502,
                message: 'Weather is temporarily unavailable. Try again.',
            },
        );
    });
}

test('search preserves query and success data', async () => {
    const api = createWeatherClient('http://localhost', async (request) => {
        assert.equal(new URL(request.url).searchParams.get('q'), 'San José');
        return json({ data: [{ id: 1 }] });
    });
    const result = await api.GET('/v1/cities', { params: { query: { q: 'San José' } } });
    assert.equal(result.data.data[0].id, 1);
});

test('new requests and explicit cancellation invalidate older completions', () => {
    const slot = new LatestRequest();
    const old = slot.start();
    const current = slot.start();
    assert.equal(old.signal.aborted, true);
    assert.equal(slot.isCurrent(old), false);
    assert.equal(slot.isCurrent(current), true);
    slot.cancel();
    assert.equal(current.signal.aborted, true);
    assert.equal(slot.isCurrent(current), false);
});

test('malformed successful JSON fails instead of presenting an invented report', async () => {
    const api = createWeatherClient(
        'http://localhost',
        async () => new Response('<html>not weather</html>'),
    );
    await assert.rejects(
        api.GET('/v1/weather', { params: { query: { city: 'Seattle, WA' } } }),
        SyntaxError,
    );
});

test('aborted fetch rejects and preserves AbortError', async () => {
    const api = createWeatherClient('http://localhost', async (request) => {
        request.signal.throwIfAborted();
    });
    const controller = new AbortController();
    controller.abort();
    await assert.rejects(
        api.GET('/v1/cities', { params: { query: { q: 'Seattle' } }, signal: controller.signal }),
        { name: 'AbortError' },
    );
});
