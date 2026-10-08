import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const spec = JSON.parse(readFileSync(new URL('../openapi.json', import.meta.url), 'utf8'));
const cases = JSON.parse(
    readFileSync(new URL('../tests/fixtures/city-query-inputs.json', import.meta.url), 'utf8'),
);

for (const flags of ['', 'u']) {
    for (const [route, name] of [
        ['/v1/cities', 'q'],
        ['/v1/weather', 'city'],
        ['/v1/forecast/hourly', 'city'],
        ['/v1/alerts', 'city'],
    ]) {
        test(`${route} city schema accepts trimmed scalar boundaries with regex flags '${flags}'`, () => {
            const { schema } = spec.paths[route].get.parameters.find((p) => p.name === name);
            for (const { name: label, query, valid } of cases) {
                const length = [...query].length;
                const accepted =
                    (schema.minLength === undefined || length >= schema.minLength) &&
                    (schema.maxLength === undefined || length <= schema.maxLength) &&
                    (schema.pattern === undefined || new RegExp(schema.pattern, flags).test(query));
                assert.equal(accepted, valid, label);
            }
        });
    }
}
