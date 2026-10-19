import assert from 'node:assert/strict';
import fs from 'node:fs/promises';

const [base, trace, output, playwright, executablePath] = process.argv.slice(2);
const { chromium } = await import(playwright);
const report = { requests: [], errors: [] };
const browser = await chromium.launch({ executablePath, args: ['--no-sandbox'] });
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  page.on('request', request => { if (request.url().includes('/api/')) report.requests.push(request.url()); });
  page.on('pageerror', error => report.errors.push(error.message));
  await page.goto(base, { waitUntil: 'networkidle' });
  assert(!report.requests.some(url => /stats\/llm-requests\/activity|cache\/clear/.test(url)));
  if (!(await page.locator('.trace-row').count())) {
    await page.locator('.sidebar-nav-item').filter({ hasText: /Traces|追踪|轨迹/ }).first().click();
  }
  await page.locator(`.trace-row[data-trace-id="${trace}"]`).click();
  await page.locator('#trace-view-waterfall-tab').click();
  await page.locator('.idle-controls').waitFor();
  await page.waitForLoadState('networkidle');
  assert(!report.requests.some(url => /idle-intervals|action-tree|content\/llm-request/.test(url)),
    'waterfall initial view fetched idle history, tree or request bodies');
  await page.locator('.idle-page-size').fill('1');
  const firstResponse = page.waitForResponse(response => response.url().includes('/idle-intervals'));
  await page.locator('.idle-load').click();
  const first = await (await firstResponse).json();
  assert.equal(first.idle_intervals.length, 1);
  assert(first.has_more);
  await page.waitForFunction(() => document.querySelector('.idle-controls')?.dataset.page === '1');
  const nextResponse = page.waitForResponse(response => response.url().includes('/idle-intervals')
    && new URL(response.url()).searchParams.get('cursor') === first.next_cursor);
  await page.locator('.idle-next').click();
  const second = await (await nextResponse).json();
  assert.equal(second.idle_intervals.length, 1);
  assert.notEqual(second.idle_intervals[0].id, first.idle_intervals[0].id);
  await page.waitForFunction(() => document.querySelector('.idle-controls')?.dataset.page === '2');
  assert.equal(await page.locator('.idle-controls').getAttribute('data-count'), '1', 'idle pages accumulated');
  const secondLane = page.locator('.wf-idle-lane').filter({
    has: page.locator(`.wf-gutter[title="Session: ${second.idle_intervals[0].session_id}"]`),
  });
  await secondLane.locator('.wf-idle-segment').first().waitFor({ state: 'visible' });
  assert.equal(report.errors.length, 0);
  await page.screenshot({ path: `${output}/idle-waterfall.png`, fullPage: true });
  report.status = 'passed';
} catch (error) {
  report.status = 'failed'; report.error = error.stack;
  throw error;
} finally {
  await browser.close();
  await fs.writeFile(`${output}/browser.json`, JSON.stringify(report, null, 2));
}
