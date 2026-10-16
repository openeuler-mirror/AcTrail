// Verify real captured long strings through bounded API pages and browser navigation.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { TimelineBrowser } from '../browser/timeline.mjs';

class LongContentAcceptance {
  constructor(base, trace, output, expected) {
    Object.assign(this, { base, trace, output, expected });
    this.endpoint = `/api/traces/${trace}/actions/${encodeURIComponent(expected.action_id)}/content/llm-request/node`;
    this.report = { requests: [], pages: [], status: 'failed' };
  }

  async read(offset) {
    const query = new URLSearchParams({ pointer: this.expected.pointer, offset, limit: 50 });
    const response = await fetch(this.base + this.endpoint + '?' + query);
    assert.equal(response.status, 200, await response.clone().text());
    return (await response.json()).content.node;
  }

  async api() {
    const bytes = Buffer.from(this.expected.value);
    assert(bytes.length > 150 * 1024);
    const prefixResponse = await fetch(this.base + this.endpoint.replace(/\/node$/, '') + '?max_bytes=131072');
    assert.equal(prefixResponse.status, 200);
    const prefixContent = (await prefixResponse.json()).content;
    assert(prefixContent.truncated, 'real request must exceed the former 128 KiB preview');
    let offset = 0;
    const values = [];
    for (let count = 0; count < 20; count++) {
      const node = await this.read(offset);
      assert.equal(node.offset, offset);
      assert.equal(node.value_bytes, bytes.length);
      assert(node.returned_bytes > 0 && node.returned_bytes <= 64 * 1024);
      assert.equal(Buffer.byteLength(node.value), node.returned_bytes);
      assert.deepEqual(Buffer.from(node.value), bytes.subarray(offset, offset + node.returned_bytes));
      assert.equal(node.truncated, offset > 0 || node.next_offset != null);
      assert.equal(node.has_more, node.next_offset != null);
      this.report.pages.push({ offset, bytes: node.returned_bytes, next_offset: node.next_offset });
      values.push(node.value);
      if (!node.has_more) {
        assert.equal(node.next_offset, null);
        assert.equal(offset + node.returned_bytes, bytes.length);
        break;
      }
      assert.equal(node.next_offset, offset + node.returned_bytes);
      offset = node.next_offset;
    }
    assert.equal(values.join(''), this.expected.value, 'page concatenation changed persisted UTF-8 content');
    this.values = values;
    const unicode = bytes.findIndex(byte => byte >= 0x80);
    assert(unicode >= 0);
    for (const invalid of [unicode + 1, bytes.length + 1]) {
      const query = new URLSearchParams({ pointer: this.expected.pointer, offset: invalid, limit: 50 });
      const response = await fetch(this.base + this.endpoint + '?' + query);
      assert.equal(response.status, 400, 'invalid string offset must fail locally');
    }
    assert.equal((await fetch(`${this.base}/api/traces/${this.trace}/metadata`)).status, 200);
    assert.equal((await this.read(0)).value, values[0], 'invalid offset affected subsequent reads');
  }

  async browser(playwright, executablePath) {
    const { chromium } = await import(playwright);
    this.instance = await chromium.launch({ executablePath, args: ['--no-sandbox'] });
    const page = await this.instance.newPage({ viewport: { width: 1440, height: 1000 } });
    page.on('request', request => {
      if (request.url().includes('/api/')) this.report.requests.push(request.url());
    });
    const timeline = new TimelineBrowser(page, this.base, this.trace);
    await timeline.open();
    await timeline.kinds(['llm.request']);
    const response = await fetch(`${this.base}/api/traces/${this.trace}/actions/${encodeURIComponent(this.expected.action_id)}`);
    assert.equal(response.status, 200);
    await timeline.select(await response.json());
    const mode = page.locator('.request-mode-switch');
    const summary = page.locator('.llm-request-summary');
    await mode.waitFor();
    assert(await summary.isVisible());
    assert(!this.contentRequests().length, 'opening detail fetched body without expansion');
    const root = page.waitForResponse(response => {
      const url = new URL(response.url());
      return url.pathname === this.endpoint && url.searchParams.get('pointer') === '';
    });
    await mode.locator('.toggle-track').click();
    assert(await mode.locator('input').isChecked());
    const rootResponse = await root;
    assert.equal(rootResponse.status(), 200);
    const rootNode = (await rootResponse.json()).content.node;
    const body = page.locator('.llm-request-blocks');
    await body.waitFor();
    await page.waitForLoadState('networkidle');
    const topPointer = '/' + this.expected.pointer.split('/')[1];
    await body.locator(`.llm-content-node[data-pointer="${topPointer}"]`).waitFor();
    assert(!(await summary.isVisible()), 'Detail retained Summary');
    assert.equal(this.contentRequests().filter(url => new URL(url).searchParams.get('pointer') === '').length,
      1, 'opening Detail must read root descriptions once');
    const initialPointers = new Set(['', ...rootNode.children
      .filter(field => !['object', 'array'].includes(field.type)).map(field => field.pointer)]);
    assert(this.contentRequests().every(url => initialPointers.has(new URL(url).searchParams.get('pointer'))),
      'opening Detail loaded an unexpanded container');
    const beforeToggle = this.contentRequests().length;
    await mode.locator('.toggle-track').click();
    assert(!(await body.isVisible()), 'request insight hide left structured content visible');
    assert(await summary.isVisible());
    await mode.locator('.toggle-track').click();
    await body.waitFor();
    await page.waitForLoadState('networkidle');
    assert.equal(this.contentRequests().length, beforeToggle, 'showing existing insights reloaded nodes');
    let pointer = '';
    for (const token of this.expected.pointer.split('/').slice(1)) {
      pointer += '/' + token;
      const field = body.locator(`.llm-content-node[data-pointer="${pointer}"]`);
      const parentPointer = pointer.slice(0, pointer.lastIndexOf('/'));
      if (parentPointer) {
        const more = body.locator(`.llm-content-node[data-pointer="${parentPointer}"]`)
          .locator(':scope > .llm-content-body > .block-load-controls > button');
        await page.waitForLoadState('networkidle');
        while (!(await field.count()) && await more.count()) {
          await more.click();
          await page.waitForLoadState('networkidle');
        }
      }
      await field.waitFor();
      const row = field.locator(':scope > .llm-content-heading');
      if (await row.count()) {
        if (/^\/(messages|input)\/\d+$/.test(pointer)) {
          await page.waitForLoadState('networkidle');
          assert.equal(await row.getAttribute('aria-expanded'), 'false');
          assert(!this.contentRequests().some(url => new URL(url).searchParams.get('pointer') === pointer + '/content'),
            'closed message card fetched its body');
        }
        if (await row.getAttribute('aria-expanded') !== 'true') await row.click();
        if (['/messages', '/input'].includes(pointer)) {
          await page.waitForLoadState('networkidle');
          assert(await field.locator(':scope > .llm-content-body > .llm-content-node').count() <= 5,
            'message list loaded more than its first five cards');
        }
      } else if (!(await field.getAttribute('class')).split(' ').includes('is-container')) {
        await field.locator(':scope > .llm-content-value').waitFor();
      }
    }
    const leaf = body.locator(`.llm-content-node[data-pointer="${this.expected.pointer}"]`);
    await leaf.locator('.llm-content-value').waitFor();
    await this.checkValue(leaf, this.values[0]);
    await page.waitForLoadState('networkidle');
    assert(await leaf.locator('.llm-content-previous').isDisabled());
    for (let index = 1; index < this.values.length; index++) {
      await this.navigate(page, leaf, 'next', this.report.pages[index].offset);
      await this.checkValue(leaf, this.values[index]);
    }
    assert(await leaf.locator('.llm-content-next').isDisabled(), 'last page still offers Next');
    for (let index = this.values.length - 2; index >= 0; index--) {
      await this.navigate(page, leaf, 'previous', this.report.pages[index].offset);
      await this.checkValue(leaf, this.values[index]);
    }
    assert(await leaf.locator('.llm-content-previous').isDisabled());
    assert(this.contentRequests().every(url => new URL(url).pathname.endsWith('/node')),
      'browser fetched a complete body instead of requested nodes');
    await page.screenshot({ path: `${this.output}/long-content.png`, fullPage: true });
  }

  contentRequests() {
    return this.report.requests.filter(url => url.includes('/content/llm-request'));
  }

  async navigate(page, leaf, direction, offset) {
    const start = this.contentRequests().length;
    const response = page.waitForResponse(response => {
      const url = new URL(response.url());
      return url.pathname === this.endpoint && url.searchParams.get('pointer') === this.expected.pointer
        && Number(url.searchParams.get('offset')) === offset;
    });
    await leaf.locator(`.llm-content-${direction}`).click();
    assert.equal((await response).status(), 200);
    await page.waitForLoadState('networkidle');
    assert.equal(this.contentRequests().length - start, 1, 'page navigation fetched unrelated content');
  }

  async checkValue(leaf, expected) {
    const value = leaf.locator('.llm-content-value');
    await value.page().waitForFunction(({ pointer, text }) => {
      const node = [...document.querySelectorAll('.llm-content-node')].find(node => node.dataset.pointer === pointer);
      return node?.querySelector('.llm-content-value')?.textContent === text;
    }, { pointer: this.expected.pointer, text: expected });
    assert.equal(await value.count(), 1, 'leaf retained previous page elements');
    assert.equal(await value.textContent(), expected, 'visible page was appended or changed');
    assert(Buffer.byteLength(await leaf.textContent()) < 70 * 1024, 'leaf DOM accumulated earlier pages');
  }
}

const [base, trace, output, playwright, executable] = process.argv.slice(2);
const expected = JSON.parse(await fs.readFile(`${output}/long-content-expected.json`, 'utf8'));
const acceptance = new LongContentAcceptance(base, trace, output, expected);
try {
  await acceptance.api();
  await acceptance.browser(playwright, executable);
  acceptance.report.status = 'passed';
} catch (error) {
  acceptance.report.error = error.stack;
  throw error;
} finally {
  await acceptance.instance?.close();
  await fs.writeFile(`${output}/long-content.json`, JSON.stringify(acceptance.report, null, 2));
}
