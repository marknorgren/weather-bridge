import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const spec = JSON.parse(readFileSync(new URL('../openapi.json', import.meta.url), 'utf8'));
const cases = JSON.parse(
    readFileSync(new URL('../tests/fixtures/city-query-inputs.json', import.meta.url), 'utf8'),
);

// Keep this reviewed constant aligned with city_query_schema in src/cities.rs.
// Check the contract as data before exercising it; never compile a schema string.
const reviewedPattern = String.raw`^[\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]*(?:[\uD800-\uDBFF][\uDC00-\uDFFF]|[^\uD800-\uDFFF\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000])(?:[\uD800-\uDBFF][\uDC00-\uDFFF]|[^\uD800-\uDFFF]){0,118}(?:[\uD800-\uDBFF][\uDC00-\uDFFF]|[^\uD800-\uDFFF\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000])[\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]*(?![\s\S])`;

for (const flags of ['', 'u']) {
    const pattern = new RegExp(reviewedPattern, flags);
    for (const [route, name] of [
        ['/v1/cities', 'q'],
        ['/v1/weather', 'city'],
        ['/v1/forecast/hourly', 'city'],
        ['/v1/alerts', 'city'],
    ]) {
        test(`${route} city schema accepts trimmed scalar boundaries with regex flags '${flags}'`, () => {
            const { schema } = spec.paths[route].get.parameters.find((p) => p.name === name);
            assert.equal(
                schema.pattern,
                pattern.source,
                'Review changes to the advertised city pattern',
            );
            for (const { name: label, query, valid } of cases) {
                const length = [...query].length;
                const accepted =
                    (schema.minLength === undefined || length >= schema.minLength) &&
                    (schema.maxLength === undefined || length <= schema.maxLength) &&
                    pattern.test(query);
                assert.equal(accepted, valid, label);
            }
        });
    }
}
