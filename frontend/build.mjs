import { readFile, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import openapiTS, { astToString } from 'openapi-typescript';
import { build } from 'esbuild';

const root = new URL('../', import.meta.url);
const schema =
    '// Generated from openapi.json by pnpm run generate. Do not edit.\n' +
    astToString(await openapiTS(new URL('openapi.json', root), { alphabetize: true }));
const clientLicense = await readFile(new URL('node_modules/openapi-fetch/LICENSE', root), 'utf8');
const result = await build({
    absWorkingDir: fileURLToPath(root),
    entryPoints: ['web/weather.ts'],
    bundle: true,
    write: false,
    format: 'iife',
    target: ['es2022'],
    legalComments: 'inline',
    // Keep bundled module paths as node_modules/<package>/... under pnpm's isolated layout.
    preserveSymlinks: true,
    banner: {
        js:
            '// Generated from web/weather.ts by pnpm run generate. Do not edit.\n/*! openapi-fetch\n' +
            clientLicense.trim() +
            '\n*/',
    },
});
const artifacts = [
    ['frontend/schema.d.ts', schema],
    ['web/weather.js', result.outputFiles[0].text],
];
for (const [path, generated] of artifacts) {
    if (process.argv.includes('--check')) {
        let current;
        try {
            current = await readFile(new URL(path, root), 'utf8');
        } catch {
            current = undefined;
        }
        if (current !== generated) {
            console.error(`${path} is stale. Run pnpm run generate.`);
            process.exitCode = 1;
        }
    } else {
        await writeFile(new URL(path, root), generated);
        console.log(`Generated ${path}`);
    }
}
