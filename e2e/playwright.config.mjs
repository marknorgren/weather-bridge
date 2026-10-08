import { fileURLToPath } from 'node:url';
import { defineConfig } from '@playwright/test';

export default defineConfig({
    testDir: '.',
    testMatch: '*.spec.mjs',
    outputDir: '../target/browser-test/results',
    reporter: [
        ['list'],
        ['html', { outputFolder: '../target/browser-test/report', open: 'never' }],
    ],
    timeout: 20000,
    fullyParallel: true,
    workers: 2,
    retries: 0,
    use: {
        baseURL: 'http://127.0.0.1:4179',
        reducedMotion: 'reduce',
        trace: 'retain-on-failure',
        screenshot: 'only-on-failure',
    },
    projects: ['chromium', 'webkit'].flatMap((browserName) => [
        {
            name: `${browserName}-desktop`,
            use: { browserName, viewport: { width: 1440, height: 1000 } },
        },
        {
            name: `${browserName}-mobile`,
            use: {
                browserName,
                viewport: { width: 390, height: 844 },
                isMobile: true,
                hasTouch: true,
            },
        },
    ]),
    webServer: {
        command: 'node e2e/server.mjs',
        cwd: fileURLToPath(new URL('../', import.meta.url)),
        url: 'http://127.0.0.1:4179',
        reuseExistingServer: false,
    },
});
