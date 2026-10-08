// Browser regression gate for the built Pages artifact. No live API required.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const http = require('node:http');
const path = require('node:path');
const { chromium, webkit } = require('@playwright/test');

const root = path.resolve(__dirname, '../target/docs-site');
const types = { '.html': 'text/html', '.css': 'text/css', '.js': 'text/javascript', '.json': 'application/json' };
const server = http.createServer(async (req, res) => {
  try {
    const name = new URL(req.url, 'http://localhost').pathname.slice(1) || 'index.html';
    if (name.includes('/') || name.includes('..')) throw new Error('Invalid path');
    res.setHeader('Content-Type', types[path.extname(name)] || 'text/plain');
    res.end(await fs.readFile(path.join(root, name)));
  } catch {
    res.writeHead(404).end();
  }
});

(async () => {
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  try {
    for (const engine of (process.env.DOCS_TEST_CHROMIUM_ONLY ? [chromium] : [chromium, webkit])) {
      const browser = await engine.launch(engine === chromium && process.env.DOCS_TEST_CHROMIUM_EXECUTABLE ? { executablePath: process.env.DOCS_TEST_CHROMIUM_EXECUTABLE } : {});
      try {
        for (const viewport of [{ width: 1440, height: 1000 }, { width: 600, height: 900 }]) {
          const page = await browser.newPage({ viewport });
          const errors = [];
          page.on('pageerror', error => errors.push(error.message));
          await page.route('**/*', async route => {
            const url = new URL(route.request().url());
            if (process.env.DOCS_TEST_STYLES && url.pathname === '/styles.css') {
              return route.fulfill({ contentType: 'text/css', body: await fs.readFile(process.env.DOCS_TEST_STYLES, 'utf8') });
            }
            if (url.origin === origin) return route.continue();
            if (url.origin === 'https://bridge.wx.mrkd.co' && url.pathname === '/v1/weather') {
              assert.equal(url.searchParams.get('city'), 'Minneapolis, MN');
              return route.fulfill({ contentType: 'application/json', body: JSON.stringify({ data: { location: { name: 'Minneapolis, MN' }, summary: 'Docs regression response', alertsStatus: 'checked' }, meta: { attribution: [] } }) });
            }
            return route.abort();
          });
          await page.goto(`${origin}/rest.html#GET/v1/weather`);
          await page.getByRole('button', { name: 'Test Request (get /v1/weather)', exact: true }).click();
          const client = page.getByRole('dialog', { name: 'API Client' });
          const headings = await client.locator('h2').evaluateAll(elements => elements.map(el => ({ text: el.textContent, size: parseFloat(getComputedStyle(el).fontSize), height: el.getBoundingClientRect().height })));
          for (const heading of headings) {
            assert.ok(heading.size <= 16, `${engine.name()} ${viewport.width}: oversized ${heading.text}: ${heading.size}px`);
            assert.ok(heading.height <= 32, `Heading row too tall: ${heading.text}`);
          }
          const headerHeights = await client.locator('th').evaluateAll(elements => elements.map(el => el.getBoundingClientRect().height));
          assert.ok(headerHeights.every(height => height <= 48), `Table headers gained docs padding: ${headerHeights}`);
          const layouts = await page.evaluate(() => {
            const elements = [...document.querySelectorAll('#app .grid, #app .section')];
            const read = () => elements.map(el => {
              const style = getComputedStyle(el);
              return [style.marginTop, style.marginBottom, style.paddingTop, style.borderTopWidth, style.rowGap];
            });
            const actual = read();
            const sheet = [...document.styleSheets].find(sheet => sheet.href?.endsWith('/styles.css'));
            sheet.disabled = true;
            const scalar = read();
            sheet.disabled = false;
            return { actual, scalar };
          });
          assert.ok(layouts.actual.length > 0, 'Scalar layout elements must be present');
          assert.deepEqual(layouts.actual, layouts.scalar, 'Authored docs stylesheet must not change Scalar grid/section spacing');
          const row = client.getByRole('row').filter({ has: page.getByRole('checkbox', { name: 'Include city in request', exact: true }) });
          // The generated contract provides a city example. Scalar initially shows
          // its example menu; choose Add value to open the custom-value editor.
          await row.getByRole('button', { name: 'Seattle, WA', exact: true }).click();
          await page.getByRole('menuitem', { name: 'Add value', exact: true }).click();
          const cityValue = row.getByPlaceholder('Value', { exact: true });
          await cityValue.fill('Minneapolis, MN');
          await cityValue.press('Enter');
          await client.getByRole('button', { name: 'Send get request to https://bridge.wx.mrkd.co/v1/weather', exact: true }).click();
          const response = client.getByRole('region', { name: 'Response', exact: true });
          await response.getByRole('link', { name: '200 OK', exact: true }).waitFor();
          await response.getByText('Docs regression response', { exact: false }).waitFor();
          assert.match(await response.innerText(), /Docs regression response/);
          const json = response.locator('.cm-content');
          await json.scrollIntoViewIfNeeded();
          const box = await json.boundingBox();
          assert.ok(box && box.width > 200 && box.height > 100, 'JSON response must have usable rendering space');
          assert.deepEqual(errors, [], 'No browser rendering errors');
          // Authored pages retain their own typography and table styles.
          await page.goto(`${origin}/mcp.html`);
          assert.equal(await page.locator('.wrap h2').first().evaluate(el => parseFloat(getComputedStyle(el).fontSize)), 31);
          assert.equal(await page.locator('.wrap td').first().evaluate(el => getComputedStyle(el).paddingTop), '14px');
          await page.close();
          console.log(`PASS ${engine.name()} ${viewport.width}px: client layout, JSON response, MCP guide`);
        }
      } finally {
        await browser.close();
      }
    }
  } finally {
    await new Promise(resolve => server.close(resolve));
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
