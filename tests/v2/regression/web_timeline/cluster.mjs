import { C } from './constants.mjs';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
const [base, uid, out, playwright, executablePath] = process.argv.slice(2);
const { chromium } = await import(playwright);
const prefix = `/api/traces/${encodeURIComponent(uid)}`;
const report = { requests: [], consoleErrors: [] };
let browser;
async function get(path) {
  const response = await fetch(base + path);
  assert.equal(response.status, C.http_ok, `${path}: ${await response.clone().text()}`);
  return response.json();
}
try {
  const rows = [];
  const listPageCount = Math.ceil(C.cluster_trace_ids.length / C.trace_api_page_size);
  for (let page = 1; page <= C.cluster_max_pages; page++) {
    const query = new URLSearchParams({ limit: String(C.trace_api_page_size), page: String(page) });
    const result = await get(`/api/traces?${query}`);
    assert(result.traces.length <= C.trace_api_page_size);
    rows.push(...result.traces);
    assert.equal(result.page, page);
    assert.equal(result.total, C.cluster_trace_ids.length);
    assert.equal(result.total_pages, listPageCount);
    if (page >= result.total_pages) break;
  }
  assert.equal(rows.length, C.cluster_trace_ids.length, 'imported trace pagination lost records');
  assert.equal(new Set(rows.map(row => row.id)).size, C.cluster_trace_ids.length);
  assert.deepEqual((await get(`/api/traces?limit=${C.trace_api_page_size}&page=${listPageCount}`)).traces.map(row => row.id),
    rows.slice((listPageCount - 1) * C.trace_api_page_size).map(row => row.id));
  const beyond = await get(`/api/traces?limit=${C.trace_api_page_size}&page=${listPageCount + 1}`);
  assert.deepEqual(beyond.traces, []);
  assert.equal(beyond.total, C.cluster_trace_ids.length);
  assert(rows.some(row => row.id === uid), 'UID changed after subsequent import');
  const meta = await get(prefix + '/metadata');
  assert.equal(meta.trace.id, uid);
  assert(meta.trace.local_id != null, 'local identity must be separately exposed');
  const exact = await get('/api/traces?' + new URLSearchParams({ trace_id: uid, limit: String(C.trace_api_page_size) }));
  assert.deepEqual(exact.traces.map(row => row.id), [uid]);
  assert.equal(exact.total, 1);
  assert.equal(exact.total_pages, 1);
  const named = await get('/api/traces?' + new URLSearchParams({ trace_id: uid, name: meta.trace.name, limit: String(C.trace_api_page_size) }));
  assert.deepEqual(named.traces.map(row => row.id), [uid]);
  const summary = await get(prefix + '/summary');
  assert.equal(summary.trace.id, uid);
  await get(prefix + '/alerts?limit=1');
  const timeline = await get(prefix + `/actions/timeline?limit=${C.visible_page_limit}`);
  assert(timeline.actions.length > 0, 'snapshot timeline empty');
  const action = timeline.actions[0];
  assert.equal((await get(prefix + '/actions/' + encodeURIComponent(action.id))).id, action.id);
  const request = timeline.actions.find(action => action.kind === 'llm.request');
  assert(request, 'real cluster capture has no model request');
  const contentPath = prefix + '/actions/' + encodeURIComponent(request.id) + '/content/llm-request';
  const content = (await get(contentPath + `?max_bytes=${C.cluster_content_bytes}`)).content;
  assert(Number(content.returned_bytes) <= C.cluster_content_bytes);
  assert((await get(contentPath + '/node?offset=0&limit=1')).content.node.children.length <= 1);
  assert.equal((await fetch(base + contentPath + `?max_bytes=${C.content_max_bytes + 1}`)).status, C.http_bad_request);
  await get(prefix + '/action-tree/root');
  browser = await chromium.launch({ executablePath, args: ['--no-sandbox'] });
  const page = await browser.newPage({ viewport: C.viewport });
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
  await fs.writeFile(`${out}/browser.json`, JSON.stringify(report, null, C.json_indent));
}
