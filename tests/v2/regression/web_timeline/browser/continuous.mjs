// Check continuous navigation using real stored actions and delayed real network requests.
import assert from 'node:assert/strict';
import { TimelineBrowser } from './timeline.mjs';

export class ContinuousAcceptance extends TimelineBrowser {
  constructor(page, base, trace, metadata, report) {
    super(page, base, trace);
    this.setMetadata(metadata);
    this.report = report;
  }

  async verify(actions, { requireCancellation = false } = {}) {
    const initialRequests = this.report.requests.filter(url => new URL(url).pathname === this.path);
    assert(initialRequests.length, 'visible timeline was never loaded');
    assert(initialRequests.every(url => {
      const query = new URL(url).searchParams;
      return query.has('from') && query.has('to') && Number(query.get('limit')) <= 250;
    }), 'timeline performed an unbounded initial request');
    assert.equal(await this.page.locator('.timeline-page-actions').count(), 0, 'manual page controls remain');
    const axis = this.page.locator('.flame-panel .timeline-overview-time');
    const labels = await axis.allTextContents();
    const initial = await this.viewport();
    assert(initial.spanMs <= 30001, 'first visible window exceeded 30 seconds');
    const startIndex = this.report.requests.length;
    await this.kinds(['llm.request']);
    const request = actions.find(action => action.kind === 'llm.request');
    assert(request, 'real capture has no request to select');
    await this.select(request);
    const detail = this.page.locator('.detail-panel');
    const title = await detail.locator('h2').innerText();
    await detail.locator('.request-mode-switch .toggle-track').click();
    await detail.locator('.llm-request-blocks .llm-content-node').first().waitFor();
    await this.settle();
    const messages = detail.locator('.llm-request-blocks .llm-content-node[data-pointer="/messages"], '
      + '.llm-request-blocks .llm-content-node[data-pointer="/input"], '
      + '.llm-request-blocks .llm-content-node[data-pointer="/prompt"]').first();
    const messageRow = messages.locator(':scope > .llm-content-heading');
    const expandable = await messageRow.count();
    if (expandable) await messageRow.click();
    else await messages.locator(':scope > .llm-content-value').waitFor();
    await this.settle();
    if (expandable) assert.equal(await messageRow.getAttribute('aria-expanded'), 'true');
    const contentCount = this.report.requests.filter(url => url.includes('/content/llm-request')).length;
    await this.page.locator('.flame-zoom-controls button').last().click();
    await this.settle();
    const zoomed = await this.viewport();
    assert(zoomed.spanMs < initial.spanMs, 'zoom did not change the visible interval');
    await this.cachedZoomRoundTrip();
    const counts = [];
    for (const fraction of [0.75, 0.25, 0.9, 0.1]) {
      await this.jump(this.timeAt(fraction));
      assert.deepEqual(await axis.allTextContents(), labels, 'global axis changed with loaded data');
      assert.equal(await detail.locator('h2').innerText(), title, 'window replacement cleared selected action');
      assert(await detail.locator('.llm-request-blocks').isVisible(), 'window replacement cleared expanded request');
      if (expandable) assert.equal(await messageRow.getAttribute('aria-expanded'), 'true',
        'window replacement collapsed the message tree');
      const count = await this.activityCount();
      assert(count <= 4000, 'retained actions exceeded the four-window cache budget');
      counts.push(count);
    }
    assert.equal(this.report.requests.filter(url => url.includes('/content/llm-request')).length, contentCount,
      'pan or prefetch reloaded selected request blocks');
    assert(!this.report.requests.slice(startIndex).some(url => /\/action-tree|\/summary(?:\?|$)|\/waterfall(?:\?|$)/.test(url)),
      'continuous navigation loaded a whole-trace projection');
    const ranges = this.responses.map(({ url }) => {
      const query = new URL(url).searchParams;
      return { from: BigInt(query.get('from')), to: BigInt(query.get('to')) };
    });
    const prefetch = ranges.some((range, index) => ranges.slice(0, index).some(earlier =>
      range.from <= earlier.from && range.to >= earlier.to
      && (range.from < earlier.from || range.to > earlier.to)));
    assert(prefetch, 'no surrounding range was fetched after visible request data');
    this.report.continuous = {
      initial: { ...initial, origin: String(initial.origin) },
      zoomed: { ...zoomed, origin: String(zoomed.origin) }, counts, prefetch,
    };
    this.report.continuous.cancellation = await this.cancelQuickJump();
    if (requireCancellation) assert(this.report.continuous.cancellation, 'long real trace did not exercise request cancellation');
    assert.equal(await detail.locator('h2').innerText(), title, 'quick jumps reset selected detail');
    assert(await detail.locator('.llm-request-blocks').isVisible(), 'quick jumps reset expanded messages');
    assert.equal(this.report.requests.filter(url => url.includes('/content/llm-request')).length, contentCount,
      'quick jumps reloaded selected request blocks');
    for (const { url, data } of this.responses) {
      const limit = Number(new URL(url).searchParams.get('limit'));
      assert(data.actions.length <= limit && data.scanned <= 2000, 'window response exceeded server budget');
    }
    await this.kinds('default');
  }

  async cachedZoomRoundTrip() {
    const narrow = await this.viewport();
    const center = String(narrow.origin + BigInt(Math.round((narrow.startMs + narrow.spanMs / 2) * 1e6)));
    await this.page.locator('.flame-zoom-controls button').first().click();
    await this.settle();
    const before = this.report.requests.filter(url => new URL(url).pathname === this.path).length;
    await this.page.locator('.flame-zoom-controls button').last().click();
    await this.jump(center);
    assert.equal(this.report.requests.filter(url => new URL(url).pathname === this.path).length, before,
      'returning to a complete loaded window fetched timeline data again');
    assert(await this.activityCount() <= 4000, 'zoom retained more actions than the cache budget');
  }

  timeAt(fraction) {
    const start = BigInt(this.bounds.start_time_unix_nanos);
    return String(start + BigInt(Math.round(Number(BigInt(this.bounds.end_time_unix_nanos) - start) * fraction)));
  }

  async activityCount() {
    const text = await this.page.locator('.flame-heading small').innerText();
    const count = Number(text.match(/\d+/)?.[0]);
    assert(Number.isFinite(count), 'activity count is unavailable');
    return count;
  }

  async cancelQuickJump() {
    // B is already cached. Leaving a pending A for B and returning to A must
    // issue a fresh request, even though A matches the previous attempt.
    await this.jump(this.timeAt(0.5));
    const top = await this.page.locator('.flame-panel').evaluate(element => element.offsetTop);
    const failures = [];
    const onFailed = request => {
      if (new URL(request.url()).pathname === this.path) failures.push(request.url());
    };
    this.page.on('requestfailed', onFailed);
    let release;
    const gate = new Promise(resolve => { release = resolve; });
    let entered;
    const held = new Promise(resolve => { entered = resolve; });
    let hold = true;
    const pattern = `**${this.path}?*`;
    await this.page.route(pattern, async route => {
      if (hold) { hold = false; entered(route.request().url()); await gate; }
      try { await route.continue(); } catch { /* A cancelled real request may already be detached. */ }
    });
    try {
      await this.jump(this.timeAt(0.05), { settle: false });
      const requested = await Promise.race([held, new Promise(resolve => setTimeout(() => resolve(false), 1500))]);
      if (!requested) return false; // A short trace can already be fully present in the bounded cache.
      assert.equal(await this.page.locator('.flame-panel').evaluate(element => element.offsetTop), top,
        'loading status moved the graph vertically');
      await this.jump(this.timeAt(0.5), { settle: false });
      await new Promise(resolve => setTimeout(resolve, 250));
      const restarted = this.page.waitForResponse(response => response.url() === requested && response.status() === 200);
      await this.jump(this.timeAt(0.05), { settle: false });
      await restarted;
      release();
      await this.settle();
      const final = await this.viewport();
      assert(final.left <= 0.06, 'late response moved viewport away from the returned range');
      assert.equal(await this.page.locator('.flame-panel').evaluate(element => element.offsetTop), top,
        'completed loading changed graph position');
      assert(failures.length > 0, 'abandoned real timeline request was not cancelled');
      return true;
    } finally {
      release();
      await this.page.unroute(pattern);
      this.page.off('requestfailed', onFailed);
    }
  }
}
