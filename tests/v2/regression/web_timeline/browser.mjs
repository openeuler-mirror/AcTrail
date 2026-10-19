// Exercise real captured traces through bounded APIs and continuous timeline navigation.
import fs from 'node:fs/promises';
import assert from 'node:assert/strict';
import { ContinuousAcceptance } from './browser/continuous.mjs';
const [base, traceId, out, endpointTemplate, playwrightPath, executablePath] = process.argv.slice(2);
const { chromium } = await import(playwrightPath);
const endpoint = endpointTemplate.replace('{trace_id}', traceId);
const report = { requests: [], httpPages: [], consoleErrors: [] };
const expected = JSON.parse(await fs.readFile(`${out}/expected.json`, 'utf8'));
let browser;
async function get(path) {
  const start = performance.now();
  const response = await fetch(base + path);
  assert.equal(response.status, 200, `${path}: ${await response.clone().text()}`);
  const result = await response.json();
  report.httpPages.push({ path, ms: performance.now() - start, actions: result.actions?.length });
  return result;
}
async function pages(filters = {}, limit = 100) {
  const actions = [], ids = new Set(), cursors = new Set();
  let cursor;
  for (let page = 0; page < expected.ids.length + 2; page++) {
    const query = new URLSearchParams({ limit: String(limit), ...filters });
    if (cursor) query.set('cursor', cursor);
    const data = await get(`${endpoint}?${query}`);
    assert(Array.isArray(data.actions), 'timeline response must contain actions');
    assert(data.actions.length <= limit, 'page exceeded requested limit');
    assert(data.scanned <= 2000, 'candidate budget exceeded');
    for (const action of data.actions) {
      assert(!ids.has(action.id), `duplicated action ${action.id}`);
      ids.add(action.id); actions.push(action);
    }
    if (!data.has_more) {
      assert(!data.next_cursor, 'terminal page retains cursor');
      return actions;
    }
    assert(data.next_cursor && !cursors.has(data.next_cursor), 'cursor did not advance');
    cursors.add(data.next_cursor); cursor = data.next_cursor;
  }
  throw new Error('pagination failed to terminate');
}
try {
  const listed = [];
  for (let i = 0; i < expected.trace_ids.length + 1; i++) {
    const query = new URLSearchParams({ limit: '1', page: String(i + 1) });
    const result = await get(`/api/traces?${query}`);
    assert(result.traces.length <= 1, 'trace list exceeded page size');
    listed.push(...result.traces.map(trace => trace.id));
    assert.equal(result.page, i + 1);
    assert.equal(result.total, expected.trace_ids.length);
    assert.equal(result.total_pages, expected.trace_ids.length);
    if (result.page >= result.total_pages) break;
  }
  assert.deepEqual(listed, expected.trace_ids, 'trace list ordering or coverage differs from SQLite');
  const lastList = await get(`/api/traces?limit=1&page=${expected.trace_ids.length}`);
  assert.deepEqual(lastList.traces.map(trace => trace.id), expected.trace_ids.slice(-1), 'direct last-page navigation failed');
  const beyondList = await get(`/api/traces?limit=1&page=${expected.trace_ids.length + 1}`);
  assert.deepEqual(beyondList.traces, []);
  assert.equal(beyondList.total, expected.trace_ids.length);
  const metadata = await get(`/api/traces/${traceId}/metadata`);
  const filter = new URLSearchParams({ trace_id: traceId, name: metadata.trace.name,
    from: metadata.trace.created_at_unix_nanos, to: metadata.trace.created_at_unix_nanos });
  const filtered = await get(`/api/traces?${filter}`);
  assert.deepEqual(filtered.traces.map(trace => String(trace.id)), [traceId], 'combined trace filters failed');
  assert.equal(filtered.total, 1);
  assert.equal(filtered.total_pages, 1);
  assert.equal((await fetch(base + '/api/traces?limit=0')).status, 400);
  assert.equal((await fetch(base + '/api/traces?page=0')).status, 400);
  const all = await pages();
  assert(metadata.timeline_bounds?.start_time_unix_nanos && metadata.timeline_bounds?.end_time_unix_nanos,
    'metadata did not expose indexed trace time bounds');
  const globalStart = BigInt(metadata.timeline_bounds.start_time_unix_nanos);
  const globalEnd = BigInt(metadata.timeline_bounds.end_time_unix_nanos);
  assert(all.every(action => BigInt(action.start_time_unix_nanos) >= globalStart
    && BigInt(action.start_time_unix_nanos) <= globalEnd), 'valid action start escaped global metadata bounds');
  assert.deepEqual(all.map(a => a.id).sort(), expected.ids.sort(), 'API pagination differs from actual SQLite valid actions');
  assert(all.length > 2, 'real trace must span multiple pages');
  assert(all.some(a => /tool|command/.test(a.kind)), 'real tool actions missing');
  const byKind = await pages({ kinds: 'llm.request' }, 2);
  assert(byKind.length > 0, 'real LLM requests missing');
  assert(byKind.every(a => a.kind === 'llm.request'), 'kind filter was not applied');
  assert.deepEqual(byKind.map(a => a.id).sort(), all.filter(a => a.kind === 'llm.request').map(a => a.id).sort());
  const selectedKinds = ['command.invocation', 'agent.invocation', 'llm.call', 'llm.request', 'llm.response'];
  const multiKind = await pages({ kinds: selectedKinds.join(',') }, 2);
  assert.deepEqual(multiKind.map(a => a.id).sort(),
    all.filter(a => selectedKinds.includes(a.kind)).map(a => a.id).sort(), 'multi-kind merge lost actions');
  const sample = all.find(a => a.process?.process_id != null);
  assert(sample, 'timeline must expose internal process_id');
  const byProcess = await pages({ process_id: String(sample.process.process_id) });
  assert(byProcess.length > 0 && byProcess.every(a => String(a.process?.process_id) === String(sample.process.process_id)), 'process filter mismatch');
  const interval = all.find(a => a.start_time_unix_nanos && a.end_time_unix_nanos
    && BigInt(a.end_time_unix_nanos) > BigInt(a.start_time_unix_nanos) + 2n);
  assert(interval, 'real trace must contain a completed interval');
  const from = BigInt(interval.start_time_unix_nanos) + 1n;
  const to = BigInt(interval.end_time_unix_nanos) - 1n;
  const inside = await pages({ from: String(from), to: String(to) });
  assert(!inside.some(a => a.id === interval.id), 'start-window query returned an earlier crossing action');
  assert.deepEqual(inside.map(a => a.id).sort(), all.filter(a => (
    BigInt(a.start_time_unix_nanos) >= from && BigInt(a.start_time_unix_nanos) <= to
  )).map(a => a.id).sort(), 'indexed start-time window differs from persisted actions');
  const boundary = BigInt(interval.start_time_unix_nanos);
  for (const [lower, upper] of [[boundary - 1n, boundary], [boundary, boundary + 1n]]) {
    const exact = await pages({ from: String(lower), to: String(upper) });
    assert(exact.some(a => a.id === interval.id), 'closed window omitted its exact boundary');
    assert(exact.every(a => BigInt(a.start_time_unix_nanos) >= lower && BigInt(a.start_time_unix_nanos) <= upper));
  }
  report.totalActions = all.length;
  const firstByKind = new Map();
  for (const action of all) if (!firstByKind.has(action.kind)) firstByKind.set(action.kind, action);
  const terminalFirst = [...firstByKind.values()].find(a => a.end_time_unix_nanos);
  assert(terminalFirst, 'need a real completed first action for indexed seek check');
  const seekFrom = String(BigInt(terminalFirst.start_time_unix_nanos) + 1n);
  await fs.writeFile(`${out}/small-budget-case.json`, JSON.stringify({
    kind: terminalFirst.kind, from: seekFrom,
    ids: all.filter(a => a.kind === terminalFirst.kind && BigInt(a.start_time_unix_nanos) >= BigInt(seekFrom))
      .map(a => a.id).sort(),
  }));
  const invalid = await fetch(base + endpoint + '?limit=0');
  assert.equal(invalid.status, 400, 'invalid limit must be rejected');
  const overLimit = await fetch(base + endpoint + '?limit=1001');
  assert.equal(overLimit.status, 400, 'oversized limit must be rejected');
  browser = await chromium.launch({ executablePath, args: ['--no-sandbox'] });
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
  const page = await context.newPage();
  page.on('request', request => { if (request.url().includes('/api/')) report.requests.push(request.url()); });
  page.on('pageerror', error => report.consoleErrors.push(error.message));
  await page.goto(base, { waitUntil: 'networkidle' });
  assert(!report.requests.some(url => url.includes('/stats/llm-requests/activity') || url.includes('/cache/clear')),
    'initial workspace triggered global statistics or cache invalidation');
  const createdMs = Math.floor(Number(BigInt(metadata.trace.created_at_unix_nanos) / 1000000n) / 60000) * 60000;
  const [activityFrom, activityTo, editedTo] = await page.evaluate(timestamp => {
    const local = millis => {
      const date = new Date(millis);
      return new Date(millis - date.getTimezoneOffset() * 60000).toISOString().slice(0, 16);
    };
    return [local(timestamp - 60000), local(timestamp + 3600000), local(timestamp + 7200000)];
  }, createdMs);
  await page.locator('[name=activity-from]').fill(activityFrom);
  await page.locator('[name=activity-to]').fill(activityTo);
  assert(!report.requests.some(url => url.includes('/stats/llm-requests/activity')), 'editing dates triggered statistics');
  const activityResponse = page.waitForResponse(response => response.url().includes('/stats/llm-requests/activity'));
  await page.locator('.dashboard-activity-load').click();
  const activity = await activityResponse;
  assert.equal(activity.status(), 200);
  const activityQuery = new URL(activity.url()).searchParams;
  assert.equal(Number(activityQuery.get('from_ms')), createdMs - 60000);
  assert.equal(Number(activityQuery.get('to_ms')), createdMs + 3600000);
  await page.waitForLoadState('networkidle');
  assert.equal(report.requests.filter(url => url.includes('/stats/llm-requests/activity')).length, 1,
    'one explicit statistics load triggered duplicate aggregation');
  const loadedRange = await page.locator('.dashboard-activity-range').innerText();
  assert(!loadedRange.includes('Invalid Date'));
  await page.locator('[name=activity-to]').fill(editedTo);
  assert.equal(await page.locator('.dashboard-activity-range').innerText(), loadedRange,
    'editing draft dates relabeled an existing result');
  assert.equal(report.requests.filter(url => url.includes('/stats/llm-requests/activity')).length, 1);
  const refreshedActivity = page.waitForResponse(response => response.url().includes('/stats/llm-requests/activity'));
  await page.locator('button.icon-button[title="Refresh"], button.icon-button[title="刷新"]').click();
  const refresh = await refreshedActivity;
  assert.equal(refresh.status(), 200);
  assert.equal(new URL(refresh.url()).searchParams.get('from_ms'), activityQuery.get('from_ms'));
  assert.equal(new URL(refresh.url()).searchParams.get('to_ms'), activityQuery.get('to_ms'));
  await page.waitForLoadState('networkidle');
  assert.equal(report.requests.filter(url => url.includes('/stats/llm-requests/activity')).length, 2,
    'explicit refresh duplicated statistics or used unsubmitted draft dates');
  const traceRequestStart = report.requests.length;
  if (!(await page.locator('.trace-row').count())) {
    await page.locator('.sidebar-nav-item').filter({ hasText: /Traces|追踪|轨迹/ }).first().click();
  }
  await page.locator('.trace-row').filter({ hasText: metadata.trace.display_id }).first().click();
  await page.locator('#trace-view-flame_graph-tab').click();
  await page.locator('.flame-panel').waitFor({ state: 'visible', timeout: 30000 });
  await page.waitForLoadState('networkidle');
  assert(report.requests.some(url => url.includes(`/api/traces/${traceId}/actions/timeline`)), 'flamegraph never requested timeline');
  assert(!report.requests.some(url => url.includes('/action-tree')), 'non-tree initial page requested action tree');
  assert(!report.requests.some(url => /content\/llm-request/.test(url)), 'initial page fetched LLM bodies');
  assert(!report.requests.slice(traceRequestStart).some(url => url.includes('/stats/llm-requests/activity')),
    'entering trace workspace triggered global statistics');
  const trajectoryStart = report.requests.length;
  const trajectoryResponse = page.waitForResponse(response => response.url().includes('/llm-trajectories'));
  await page.locator('#trace-view-llm_trajectory-tab').click();
  assert.equal((await trajectoryResponse).status(), 200);
  await page.waitForLoadState('networkidle');
  assert(!report.requests.slice(trajectoryStart).some(url => /\/waterfall(?:\?|$)|\/action-tree|content\/llm-request/.test(url)),
    'trajectory view fetched whole-trace projection or request bodies');
  await page.locator('#trace-view-flame_graph-tab').click();
  await page.locator('.flame-panel').waitFor();
  for (const oldEndpoint of ['action-tree', 'waterfall']) {
    assert.equal((await fetch(`${base}/api/traces/${traceId}/${oldEndpoint}`)).status, 404);
  }
  const selectedSummary = await page.locator('.rail-summary').innerText();
  const listControls = page.locator('.trace-list-controls');
  await listControls.locator('.trace-filter-toggle').click();
  await listControls.locator('.trace-filter-form').waitFor();
  const listRequestStart = report.requests.length;
  const firstListResponse = page.waitForResponse(response => {
    const url = new URL(response.url());
    return url.pathname === '/api/traces' && url.searchParams.get('limit') === '10'
      && url.searchParams.get('page') === '1';
  });
  await listControls.locator('.trace-page-size select').selectOption('10');
  const firstList = await (await firstListResponse).json();
  await page.waitForLoadState('networkidle');
  assert.equal(await page.locator('.trace-row').count(), Math.min(10, firstList.total), 'list page was appended instead of replaced');
  assert.equal(await page.locator('.rail-summary').innerText(), selectedSummary, 'list filter changed selected metadata');
  const targetPage = firstList.total_pages > 1 ? firstList.total_pages : 1;
  const nextListResponse = page.waitForResponse(response => {
    const url = new URL(response.url());
    return url.pathname === '/api/traces' && url.searchParams.get('page') === String(targetPage)
      && url.searchParams.get('limit') === '10';
  });
  await listControls.locator('.trace-page-input').fill(String(targetPage));
  await listControls.locator('.trace-page-go').click();
  const secondList = await (await nextListResponse).json();
  await page.waitForLoadState('networkidle');
  assert.equal(await page.locator('.trace-row').count(), secondList.traces.length, 'trace page was not replaced');
  assert.equal(secondList.page, targetPage);
  if (targetPage > 1) assert.notEqual(String(firstList.traces[0].id), String(secondList.traces[0].id));
  report.traceListNavigation = { total: firstList.total, page: targetPage, crossedPage: targetPage > 1 };
  assert.equal(await page.locator('.rail-summary').innerText(), selectedSummary, 'paging changed selected metadata');
  assert(await page.locator('.flame-panel').isVisible(), 'paging hid selected trace view');
  assert(!report.requests.slice(listRequestStart).some(url => /\/api\/traces\/[^/]+\/(metadata|actions\/timeline)/.test(new URL(url).pathname)),
    'list navigation unexpectedly reloaded or changed selected trace');
  const restoredListResponse = page.waitForResponse(response => {
    const url = new URL(response.url());
    return url.pathname === '/api/traces' && url.searchParams.get('limit') === '50'
      && url.searchParams.get('page') === '1';
  });
  await listControls.locator('.trace-page-size select').selectOption('50');
  await restoredListResponse;
  await page.waitForLoadState('networkidle');
  assert.equal(await page.locator('.rail-summary').innerText(), selectedSummary);
  const continuous = new ContinuousAcceptance(page, base, traceId, metadata, report);
  await continuous.verify(all);
  await page.locator('#trace-view-action_tree-tab').click();
  await page.waitForLoadState('networkidle');
  const beforeReturn = report.requests.length;
  await page.locator('#trace-view-flame_graph-tab').click();
  await page.locator('.flame-panel').waitFor();
  await page.waitForLoadState('networkidle');
  assert(!report.requests.slice(beforeReturn).some(url => url.includes('/action-tree')
    || /content\/llm-request/.test(url)), 'returning to flamegraph fetched tree or bodies');
  await page.screenshot({ path: `${out}/flamegraph.png`, fullPage: true });
  await page.locator('#trace-view-waterfall-tab').click();
  const row = page.locator('.wf-row[data-action-id]').first();
  await row.waitFor({ state: 'visible' });
  const actionId = await row.getAttribute('data-action-id');
  const detailPath = `/api/traces/${traceId}/actions/${encodeURIComponent(actionId)}`;
  const detailResponse = page.waitForResponse(response => new URL(response.url()).pathname === detailPath);
  await row.click();
  assert.equal((await detailResponse).status(), 200, 'waterfall detail request failed');
  await page.locator('.detail-panel').waitFor({ state: 'visible' });
  await page.waitForLoadState('networkidle');
  assert.equal(await page.locator('.detail-panel .detail-error').count(), 0, 'waterfall detail panel displays an error');
  assert.equal(report.consoleErrors.length, 0, `browser errors: ${report.consoleErrors.join('; ')}`);
  await page.screenshot({ path: `${out}/waterfall-detail.png`, fullPage: true });
  report.status = 'passed';
} catch (error) {
  report.status = 'failed'; report.error = error.stack;
  throw error;
} finally {
  await fs.writeFile(`${out}/browser.json`, JSON.stringify(report, null, 2));
  if (browser) await browser.close();
}
