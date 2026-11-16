// Focused resource-track acceptance against an already running real Web instance.
import fs from 'node:fs/promises';
import assert from 'node:assert/strict';
import { C } from '../constants.mjs';
import { ResourceAcceptance } from './resources.mjs';

const [base, trace, out, playwrightPath, executablePath, mode = 'sampled'] = process.argv.slice(2);
const { chromium } = await import(playwrightPath);
const browser = await chromium.launch({ executablePath, args: ['--no-sandbox'] });
const report = { requests: [], consoleErrors: [], status: 'failed' };
let page;
try {
  page = await browser.newPage({ viewport: C.viewport });
  page.on('request', request => { if (request.url().includes('/api/')) report.requests.push(request.url()); });
  page.on('pageerror', error => report.consoleErrors.push(error.message));
  const resources = new ResourceAcceptance(page, base, trace, report);
  await resources.open();
  await resources.verify({ empty: mode === 'empty' });
  assert.deepEqual(report.consoleErrors, []);
  await page.screenshot({ path: `${out}/resources-${mode}.png`, fullPage: true });
  report.status = 'passed';
} catch (error) {
  report.error = error.stack;
  if (page) await page.screenshot({ path: `${out}/resources-${mode}-failed.png`, fullPage: true }).catch(() => {});
  throw error;
} finally {
  await fs.writeFile(`${out}/resources-${mode}.json`, JSON.stringify(report, null, C.json_indent));
  await browser.close();
}
