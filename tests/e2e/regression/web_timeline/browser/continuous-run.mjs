// Verify continuous window behavior on an existing real captured trace.
import { C } from '../constants.mjs';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { TimelineBrowser } from './timeline.mjs';
import { ContinuousAcceptance } from './continuous.mjs';

const [base, trace, output, playwright, executablePath] = process.argv.slice(2);
const report = { status: 'failed', requests: [], consoleErrors: [] };
let browser;
try {
  const { chromium } = await import(playwright);
  browser = await chromium.launch({ executablePath, args: ['--no-sandbox'] });
  const page = await browser.newPage({ viewport: C.viewport });
  page.on('request', request => { if (request.url().includes('/api/')) report.requests.push(request.url()); });
  page.on('pageerror', error => report.consoleErrors.push(error.message));
  const navigation = new TimelineBrowser(page, base, trace);
  await navigation.open();
  const response = await fetch(`${base}/api/traces/${encodeURIComponent(trace)}/actions/timeline?kinds=llm.request&limit=${C.filtered_page_limit}`);
  assert.equal(response.status, C.http_ok);
  const actions = (await response.json()).actions;
  const acceptance = new ContinuousAcceptance(page, base, trace, navigation.metadata, report);
  await acceptance.verify(actions, { requireCancellation: true });
  assert.deepEqual(report.consoleErrors, []);
  assert(!report.requests.some(url => /\/action-tree|\/summary(?:\?|$)|\/waterfall(?:\?|$)/.test(url)));
  await page.screenshot({ path: `${output}/continuous.png`, fullPage: true });
  report.status = 'passed';
} catch (error) {
  await browser?.contexts()[0]?.pages()[0]?.screenshot({ path: `${output}/failure.png`, fullPage: true });
  report.error = error.stack;
  throw error;
} finally {
  await browser?.close();
  await fs.writeFile(`${output}/continuous.json`, JSON.stringify(report, null, C.json_indent));
}
