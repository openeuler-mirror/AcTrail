// Verify an actual ordinary HTTP request appears and opens from default Flamegraph kinds.
import { C } from '../constants.mjs';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { TimelineBrowser } from '../browser/timeline.mjs';

class NetworkBrowserAcceptance {
  constructor(base, output, expected) {
    Object.assign(this, { base, output, expected });
    this.report = { status: 'failed', requests: [], consoleErrors: [] };
  }

  async run(playwright, executablePath) {
    const { chromium } = await import(playwright);
    this.browser = await chromium.launch({ executablePath, args: ['--no-sandbox'] });
    const page = await this.browser.newPage({ viewport: C.viewport });
    page.on('request', request => {
      if (request.url().includes('/api/')) this.report.requests.push(request.url());
    });
    page.on('pageerror', error => this.report.consoleErrors.push(error.message));
    const trace = String(this.expected.trace_id);
    const timeline = new TimelineBrowser(page, this.base, trace);
    await timeline.open();
    const initial = this.report.requests.map(url => new URL(url)).filter(url => url.pathname === timeline.path);
    assert(initial.length, 'default flamegraph did not load a timeline');
    assert(initial.every(url => url.searchParams.get('kinds')?.split(',').includes('http.message')),
      'default timeline excluded HTTP messages');
    const controls = page.locator('.timeline-controls');
    await controls.locator('.timeline-kinds-toggle').click();
    assert(await controls.locator('input[value="http.message"]').isChecked());
    assert.equal(await controls.locator('select, input:not([type=checkbox])').count(), 0,
      'timeline filters exposed query coordinates or request budgets');
    const messageToggle = page.locator('.flame-message-toggle');
    assert(await messageToggle.locator('input[type=checkbox]').isChecked());
    const beforeToggle = this.report.requests.filter(url => new URL(url).pathname === timeline.path).length;
    await messageToggle.locator('.toggle-track').click();
    assert(!(await messageToggle.locator('input[type=checkbox]').isChecked()));
    await messageToggle.locator('.toggle-track').click();
    await timeline.settle();
    assert(await controls.locator('input[value="http.message"]').isChecked(), 'message display switch changed action categories');
    assert.equal(this.report.requests.filter(url => new URL(url).pathname === timeline.path).length,
      beforeToggle, 'message display switch reloaded the timeline');
    await controls.locator('.timeline-kinds-toggle').click();
    const response = await fetch(`${this.base}/api/traces/${trace}/actions/${encodeURIComponent(this.expected.action_id)}`);
    assert.equal(response.status, C.http_ok);
    const target = await response.json();
    assert.equal(target.kind, 'http.message');
    assert.equal(target.title, this.expected.title);
    await page.locator('input[type=search]').first().fill(this.expected.path);
    const hit = await timeline.select(target, 'Network');
    const title = hit.title;
    assert(title.includes(this.expected.title), 'HTTP canvas label lost method or request path');
    await page.locator('.detail-panel').waitFor();
    assert((await page.locator('.detail-panel').innerText()).includes(this.expected.path));
    assert(!this.report.requests.some(url => url.includes('/action-tree')), 'HTTP display built an action tree');
    assert.deepEqual(this.report.consoleErrors, []);
    this.report.title = title;
    await page.screenshot({ path: `${this.output}/network-flamegraph.png`, fullPage: true });
    this.report.status = 'passed';
  }
}

const [base, output, playwright, executable] = process.argv.slice(2);
const expected = JSON.parse(await fs.readFile(`${output}/network-expected.json`, 'utf8'));
const acceptance = new NetworkBrowserAcceptance(base, output, expected);
try {
  await acceptance.run(playwright, executable);
} catch (error) {
  acceptance.report.error = error.stack;
  throw error;
} finally {
  await acceptance.browser?.close();
  await fs.writeFile(`${output}/network-browser.json`, JSON.stringify(acceptance.report, null, C.json_indent));
}
