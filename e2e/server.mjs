// An owned loopback origin for browser fixtures; this never calls NWS or production.
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';

const files = new Map([
    ['/', ['web/index.html', 'text/html']],
    ['/developer', ['web/developer.html', 'text/html']],
    ['/assets/weather.js', ['web/weather.js', 'text/javascript']],
    ['/assets/developer.js', ['web/developer.js', 'text/javascript']],
]);
const server = createServer(async (request, response) => {
    const file = files.get(new URL(request.url, 'http://localhost').pathname);
    if (!file) return response.writeHead(404).end();
    try {
        const body = await readFile(new URL('../' + file[0], import.meta.url));
        response.writeHead(200, { 'Content-Type': file[1], 'Cache-Control': 'no-store' }).end(body);
    } catch {
        response.writeHead(500).end();
    }
});
server.listen(4179, '127.0.0.1');
for (const signal of ['SIGTERM', 'SIGINT']) process.on(signal, () => server.close());
