// Compare real canonical requests and exercise bounded content navigation.
import { C } from './constants.mjs';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { TimelineBrowser } from './browser/timeline.mjs';

const [base, trace, output, phase, playwright, executablePath] = process.argv.slice(2);
const expected = JSON.parse(await fs.readFile(`${output}/content-expected.json`, 'utf8'));
const endpoint = `/api/traces/${trace}/actions/${encodeURIComponent(expected.action_id)}/content/llm-request`;
const report = { phase, requests: [] };
let browser;
const encodeToken = token => String(token).replaceAll('~', '~0').replaceAll('/', '~1');
async function get(path) {
  const response = await fetch(base + path);
  assert.equal(response.status, C.http_ok, `${path}: ${await response.clone().text()}`);
  return response.json();
}
async function node(pointer, offset = C.content_start_offset, limit = C.content_page_limit) {
  return (await get(endpoint + '/node?' + new URLSearchParams({ pointer, offset, limit }))).content;
}
function findString(value, pointer = C.json_root_pointer) {
  if (typeof value === 'string' && Buffer.byteLength(value) > C.content_preview_bytes && /[^\x00-\x7f]/.test(value)) return { value, pointer };
  if (value && typeof value === 'object') {
    for (const [key, child] of Object.entries(value)) {
      const found = findString(child, pointer + '/' + encodeToken(key));
      if (found) return found;
    }
  }
}
function prefix(value, bytes) {
  const raw = Buffer.from(value);
  let end = Math.min(raw.length, bytes);
  while (end > 0 && end < raw.length && (raw[end] & C.utf8_continuation_mask) === C.utf8_continuation_prefix) end--;
  return raw.subarray(0, end).toString('utf8');
}
try {
  if (phase === 'blocked') {
    const root = await node(C.json_root_pointer, C.content_start_offset, C.content_page_limit);
    assert.equal(root.node.type, 'object', 'root navigation unexpectedly decoded content blocks');
    const blockPointer = expected.block_pointers[0];
    const tokens = blockPointer.split('/').slice(1);
    const decodeToken = token => token.replaceAll('~1', '/').replaceAll('~0', '~');
    const childToken = decodeToken(tokens.at(-1));
    const parentPointer = '/' + tokens.slice(0, -1).join('/');
    let parent = expected.body;
    for (const token of tokens.slice(0, -1)) parent = parent[decodeToken(token)];
    const childIndex = Array.isArray(parent) ? Number(childToken) : Object.keys(parent).sort().indexOf(childToken);
    assert(childIndex >= 0, 'persisted block pointer is absent from actual parent');
    const description = (await node(tokens.length === 1 ? '' : parentPointer, childIndex, 1)).node;
    assert.equal(description.children.length, 1);
    const descriptor = description.children[0];
    assert.equal(descriptor.pointer, blockPointer);
    const storedValue = parent[childToken];
    const storedKind = storedValue === null ? 'null' : Array.isArray(storedValue) ? 'array' : typeof storedValue;
    assert.equal(descriptor.type, storedKind, 'bounded block description has the wrong actual kind');
    assert.equal(descriptor.child_count, null, 'block description unexpectedly decoded its children');
    const query = new URLSearchParams({ pointer: blockPointer, offset: '0', limit: '1' });
    const response = await fetch(base + endpoint + '/node?' + query);
    assert.equal(response.status, C.http_bad_request);
    assert.match(await response.text(), /budget|limit/i);
    await get(`/api/traces/${trace}/metadata`);
    await get(`/api/traces/${trace}/actions/timeline?limit=1`);
  } else if (phase === 'full') {
    const full = (await get(endpoint + `?max_bytes=${C.content_max_bytes}`)).content;
    assert(!full.truncated, 'real acceptance request exceeded full comparison budget');
    assert.deepEqual(JSON.parse(full.body_json), expected.body, 'content differs from persisted skeleton and blocks');
    const size = Buffer.byteLength(full.body_json);
    assert.equal(Number(full.canonical_body_bytes), size);
    const unicode = Buffer.from(full.body_json).findIndex(byte => byte >= C.utf8_continuation_prefix);
    assert(unicode >= 0, 'real request must contain UTF-8 characters');
    for (const limit of new Set([...C.content_boundary_bytes, unicode + 1, Math.floor(size / 2), size - 1, size])) {
      const result = (await get(endpoint + '?max_bytes=' + limit)).content;
      assert.equal(result.body_json, prefix(full.body_json, limit));
      assert.equal(Number(result.returned_bytes), Buffer.byteLength(result.body_json));
      assert.equal(Number(result.canonical_body_bytes), size);
      assert.equal(result.truncated, Number(result.returned_bytes) < size);
    }
    const keys = Object.keys(expected.body).sort();
    const children = [];
    for (let offset = 0; offset < keys.length; offset += C.content_page_limit) {
      const result = await node('', offset);
      assert.equal(Number(result.canonical_body_bytes), size);
      assert(result.node.children.length <= C.content_page_limit);
      assert.equal(result.node.total_children, keys.length);
      children.push(...result.node.children.map(child => child.token));
    }
    assert.deepEqual(children, keys);
    assert.equal((await fetch(base + endpoint + `?max_bytes=${C.content_max_bytes + 1}`)).status, C.http_bad_request);
    assert.equal((await fetch(base + endpoint + `/node?offset=${C.content_start_offset}&limit=${C.content_max_page_limit + 1}`)).status, C.http_bad_request);
  } else {
    const leaf = ['messages', 'input', 'system', 'prompt'].map(key =>
      findString(expected.body[key], '/' + key)).find(Boolean);
    assert(leaf, 'real request needs a nontrivial string leaf');
    const result = (await node(leaf.pointer)).node;
    assert.equal(result.value, prefix(leaf.value, C.content_preview_bytes));
    assert.equal(Number(result.value_bytes), Buffer.byteLength(leaf.value));
    assert.equal(Number(result.returned_bytes), Buffer.byteLength(result.value));
    assert(result.truncated);
    const raw = Buffer.from(leaf.value);
    const unicodeOffset = raw.findIndex(byte => byte >= C.utf8_continuation_prefix);
    const unicodePage = (await node(leaf.pointer, unicodeOffset)).node;
    assert.equal(unicodePage.value, prefix(raw.subarray(unicodeOffset).toString('utf8'), C.content_preview_bytes));
    assert.equal(unicodePage.offset, unicodeOffset);
    assert.equal(unicodePage.returned_bytes, Buffer.byteLength(unicodePage.value));
    const { chromium } = await import(playwright);
    browser = await chromium.launch({ executablePath, args: ['--no-sandbox'] });
    const page = await browser.newPage({ viewport: C.viewport });
    page.on('request', request => { if (request.url().includes('/api/')) report.requests.push(request.url()); });
    const timeline = new TimelineBrowser(page, base, trace);
    await timeline.open();
    await timeline.kinds(['llm.request']);
    const target = await get(`/api/traces/${trace}/actions/${encodeURIComponent(expected.action_id)}`);
    await timeline.select(target);
    const mode = page.locator('.request-mode-switch');
    await mode.waitFor();
    assert(await page.locator('.llm-request-summary').isVisible());
    assert(!report.requests.some(url => url.includes('/content/llm-request')), 'detail opened request body automatically');
    await mode.locator('.toggle-track').click();
    assert(await mode.locator('input').isChecked());
    const body = page.locator('.llm-request-blocks');
    await body.waitFor();
    await page.waitForLoadState('networkidle');
    const initialRequests = report.requests.filter(url => url.includes('/content/llm-request'));
    assert(!(await page.locator('.llm-request-summary').isVisible()));
    assert.equal(initialRequests.filter(url => new URL(url).searchParams.get('pointer') === '').length,
      1, 'opening Detail should read root descriptions once');
    assert.equal(new URL(initialRequests[0]).searchParams.get('pointer'), '');
    const toolsField = Array.isArray(expected.body.tools) && expected.body.tools.length ? 'tools'
      : Array.isArray(expected.body.functions) && expected.body.functions.length ? 'functions' : null;
    assert(toolsField, 'real tool task must retain tool definitions');
    const tools = body.locator(`.llm-content-node[data-pointer="/${toolsField}"]`);
    await tools.waitFor();
    assert(!report.requests.some(url => new URL(url).searchParams.get('pointer') === `/${toolsField}`),
      'tool definitions were loaded before expansion');
    const toolResponse = page.waitForResponse(response => {
      const url = new URL(response.url());
      return url.pathname.endsWith('/content/llm-request/node')
        && url.searchParams.get('pointer') === `/${toolsField}`;
    });
    // Card headers read their object descriptor on mount; observe before opening the list.
    const definitionResponse = page.waitForResponse(response => {
      const url = new URL(response.url());
      return url.pathname.endsWith('/content/llm-request/node')
        && url.searchParams.get('pointer') === `/${toolsField}/0`;
    });
    await tools.locator(':scope > .llm-content-heading').click();
    const toolNode = (await (await toolResponse).json()).content.node;
    assert.equal(toolNode.type, 'array');
    assert.equal(toolNode.total_children, expected.body[toolsField].length);
    assert(toolNode.children.length <= C.content_max_children, 'tool list exceeded its first page');
    const firstTool = tools.locator(`.llm-content-node[data-pointer="/${toolsField}/0"]`);
    await firstTool.waitFor();
    await page.waitForLoadState('networkidle');
    assert.equal(await firstTool.locator(':scope > .llm-content-heading').getAttribute('aria-expanded'), 'false');
    assert(!report.requests.some(url => {
      const pointer = new URL(url).searchParams.get('pointer');
      return pointer === `/${toolsField}/0/input_schema` || pointer === `/${toolsField}/0/parameters`;
    }), 'closed tool card fetched its parameter schema');
    await firstTool.locator(':scope > .llm-content-heading').click();
    const definition = (await (await definitionResponse).json()).content.node;
    assert.equal(definition.type, 'object');
    assert.deepEqual(definition.children.map(child => child.token).sort(),
      Object.keys(expected.body[toolsField][0]).sort(), 'tool definition differs from persisted request');
    const tokens = leaf.pointer.split('/').slice(1);
    let pointer = '';
    for (const token of tokens) {
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
      const button = field.locator(':scope > .llm-content-heading');
      if (await button.count()) {
        if (await button.getAttribute('aria-expanded') !== 'true') await button.click();
      } else if (!(await field.getAttribute('class')).split(' ').includes('is-container')) {
        await field.locator(':scope > .llm-content-value').waitFor();
      }
    }
    const preview = body.locator(`.llm-content-node[data-pointer="${leaf.pointer}"]`);
    await preview.locator(':scope > .llm-content-pages').waitFor();
    assert.equal(await preview.locator(':scope > .llm-content-value').textContent(), prefix(leaf.value, C.content_preview_bytes));
    assert(await preview.locator('.llm-content-next').isEnabled(), 'truncated text has no next page');
    assert.equal(report.requests.filter(url => url.includes('/content/llm-request')
      && new URL(url).searchParams.get('pointer') === '').length, 1, 'card expansion reloaded root');
    await page.screenshot({ path: `${output}/content-preview.png`, fullPage: true });
    assert(!report.requests.some(url => /\/content\/llm-request\?/.test(url)), 'node browser fetched complete body');
  }
  report.status = 'passed';
} catch (error) {
  report.status = 'failed'; report.error = error.stack;
  throw error;
} finally {
  if (browser) await browser.close();
  await fs.writeFile(`${output}/content-${phase}.json`, JSON.stringify(report, null, C.json_indent));
}
