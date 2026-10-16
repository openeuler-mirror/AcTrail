import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
const [base, uid, out, playwright, executablePath] = process.argv.slice(2);
const { chromium } = await import(playwright);
const prefix = `/api/traces/${encodeURIComponent(uid)}`;
const report = { requests: [], consoleErrors: [] };
let browser;
async function get(path) {
  const response = await fetch(base + path);
  assert.equal(response.status, 200, `${path}: ${await response.clone().text()}`);
  return response.json();
}
try {
  const rows = [];
  for (let page = 1; page <= 5; page++) {
    const query = new URLSearchParams({ limit: '1', page: String(page) });
    const result = await get(`/api/traces?${query}`);
    assert(result.traces.length <= 1);
    rows.push(...result.traces);
    assert.equal(result.page, page);
    assert.equal(result.total, 3);
    assert.equal(result.total_pages, 3);
    if (page >= result.total_pages) break;
  }
  assert.equal(rows.length, 3, 'imported trace pagination lost records');
  assert.equal(new Set(rows.map(row => row.id)).size, 3);
  assert.deepEqual((await get('/api/traces?limit=1&page=3')).traces.map(row => row.id), [rows[2].id]);
  const beyond = await get('/api/traces?limit=1&page=4');
  assert.deepEqual(beyond.traces, []);
  assert.equal(beyond.total, 3);
  assert(rows.some(row => row.id === uid), 'UID changed after subsequent import');
  const meta = await get(prefix + '/metadata');
  assert.equal(meta.trace.id, uid);
  assert(meta.trace.local_id != null, 'local identity must be separately exposed');
  const exact = await get('/api/traces?' + new URLSearchParams({ trace_id: uid, limit: '1' }));
  assert.deepEqual(exact.traces.map(row => row.id), [uid]);
  assert.equal(exact.total, 1);
  assert.equal(exact.total_pages, 1);
  const named = await get('/api/traces?' + new URLSearchParams({ trace_id: uid, name: meta.trace.name, limit: '1' }));
  assert.deepEqual(named.traces.map(row => row.id), [uid]);
  const summary = await get(prefix + '/summary');
  assert.equal(summary.trace.id, uid);
  await get(prefix + '/alerts?limit=1');
  const timeline = await get(prefix + '/actions/timeline?limit=250');
  assert(timeline.actions.length > 0, 'snapshot timeline empty');
  const action = timeline.actions[0];
  assert.equal((await get(prefix + '/actions/' + encodeURIComponent(action.id))).id, action.id);
  const request = timeline.actions.find(action => action.kind === 'llm.request');
  assert(request, 'real cluster capture has no model request');
  const contentPath = prefix + '/actions/' + encodeURIComponent(request.id) + '/content/llm-request';
  const content = (await get(contentPath + '?max_bytes=31')).content;
  assert(Number(content.returned_bytes) <= 31);
  assert((await get(contentPath + '/node?offset=0&limit=1')).content.node.children.length <= 1);
  assert.equal((await fetch(base + contentPath + '?max_bytes=16777217')).status, 400);
  await get(prefix + '/action-tree/root');
  browser = await chromium.launch({ executablePath, args: ['--no-sandbox'] });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  page.on('pageerror', error => report.consoleErrors.push(error.message));
  page.on('request', request => { if (request.url().includes('/api/')) report.requests.push(request.url()); });
  await page.goto(base, { waitUntil: 'networkidle' });
  assert(!report.requests.some(url => /stats\/llm-requests\/activity|cache\/clear/.test(url)));
  if (!(await page.locator('.trace-row').count())) {
    await page.locator('.sidebar-nav-item').filter({ hasText: /Traces|追踪|轨迹/ }).first().click();
  }
  await page.locator(`.trace-row[data-trace-id="${uid}"]`).click();
  await page.locator('#trace-view-flame_graph-tab').click();
  await page.locator('.flame-panel').waitFor();
  await page.waitForLoadState('networkidle');
  assert(report.requests.some(url => url.includes(prefix + '/metadata')));
  assert(report.requests.some(url => url.includes(prefix + '/actions/timeline')));
  assert(!report.requests.some(url => url.includes('/action-tree') || url.includes('/content/llm-request')));
  assert.equal(report.consoleErrors.length, 0);
  await page.screenshot({ path: `${out}/cluster-flamegraph.png`, fullPage: true });
  report.status = 'passed';
} catch (error) {
  report.status = 'failed'; report.error = error.stack;
  throw error;
} finally {
  if (browser) await browser.close();
  await fs.writeFile(`${out}/browser.json`, JSON.stringify(report, null, 2));
}
