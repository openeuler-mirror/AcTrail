// Drive real timeline controls and locate canvas actions from their persisted times.
import { C } from '../constants.mjs';
import assert from 'node:assert/strict';
import { TimelineFilters } from '../../../../../crates/apps/web/frontend/src/workspaces/timeline/filters.js';

export class TimelineBrowser {
  constructor(page, base, trace) {
    Object.assign(this, { page, base, trace: String(trace) });
    this.path = `/api/traces/${encodeURIComponent(trace)}/actions/timeline`;
    this.responses = [];
    this.pending = new Set();
    page.on('response', response => {
      if (new URL(response.url()).pathname !== this.path || response.status() !== C.http_ok) return;
      const task = response.json().then(data => this.responses.push({
        url: response.url(), data,
      })).catch(() => {}).finally(() => this.pending.delete(task));
      this.pending.add(task);
    });
  }

  async open(tab = C.default_tab) {
    const response = await fetch(`${this.base}/api/traces/${encodeURIComponent(this.trace)}/metadata`);
    assert.equal(response.status, C.http_ok);
    this.metadata = await response.json();
    this.setMetadata(this.metadata);
    await this.page.goto(this.base, { waitUntil: 'networkidle' });
    if (!(await this.page.locator('.trace-row').count())) {
      await this.page.locator('.sidebar-nav-item').filter({ hasText: /Traces|追踪|轨迹/ }).first().click();
    }
    await this.page.locator(`.trace-row[data-trace-id="${this.trace}"]`).click();
    await this.page.locator(`#trace-view-${tab}-tab`).click();
    await this.settle();
  }

  setMetadata(metadata) {
    this.metadata = metadata;
    const trace = metadata.trace;
    const starts = [metadata.timeline_bounds.start_time_unix_nanos,
      trace.started_at_unix_nanos ?? trace.created_at_unix_nanos].filter(value => value != null).map(BigInt);
    const ends = [metadata.timeline_bounds.end_time_unix_nanos, trace.completed_at_unix_nanos,
      trace.exited_at_unix_nanos, trace.failed_at_unix_nanos].filter(value => value != null).map(BigInt);
    this.bounds = {
      start_time_unix_nanos: String(starts.reduce((a, b) => a < b ? a : b)),
      end_time_unix_nanos: String(ends.reduce((a, b) => a > b ? a : b)),
    };
  }

  async settle() {
    // Wait for navigation to schedule its request before checking network idle.
    await this.page.waitForTimeout(C.navigation_settle_ms);
    await this.page.waitForLoadState('networkidle');
    await Promise.all([...this.pending]);
    await this.page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  }

  async kinds(kinds) {
    const controls = this.page.locator('.timeline-controls');
    const toggle = controls.locator('.timeline-kinds-toggle');
    if (await toggle.getAttribute('aria-expanded') !== 'true') await toggle.click();
    const selected = kinds === 'default'
      ? TimelineFilters.defaultKinds : kinds;
    assert(selected === 'all' || (Array.isArray(selected) && selected.length), 'select at least one action category');
    const checkboxes = controls.locator('input[type=checkbox]');
    if (Array.isArray(selected)) {
      for (const kind of selected) assert.equal(await controls.locator(`input[value="${kind}"]`).count(), 1);
    }
    for (const checkbox of await checkboxes.all()) {
      await checkbox.setChecked(selected === 'all' || selected.includes(await checkbox.inputValue()));
    }
    await controls.locator('button[type=submit]').click();
    await this.settle();
  }

  async viewport() {
    const window = this.page.locator('.flame-panel .timeline-overview-window');
    await window.waitFor();
    const ratio = await window.evaluate((element, constants) => ({
      left: parseFloat(element.style.left) / constants.percent_scale, width: parseFloat(element.style.width) / constants.percent_scale,
    }), C);
    const start = BigInt(this.bounds.start_time_unix_nanos);
    const span = Math.max(Number(BigInt(this.bounds.end_time_unix_nanos) - start) / C.nanoseconds_per_ms, 1);
    return { origin: start, startMs: ratio.left * span, spanMs: ratio.width * span, totalMs: span, ...ratio };
  }

  async jump(time, { settle = C.default_settle } = structuredClone(C.navigation_options)) {
    const track = this.page.locator('.flame-panel .timeline-overview-track');
    await track.scrollIntoViewIfNeeded();
    const box = await track.boundingBox();
    const start = BigInt(this.bounds.start_time_unix_nanos);
    const span = BigInt(this.bounds.end_time_unix_nanos) - start;
    const ratio = span > 0n ? Number(BigInt(time) - start) / Number(span) : 0;
    await track.click({ position: { x: Math.max(1, Math.min(box.width - 1, ratio * box.width)), y: C.overview_click_y } });
    if (settle) await this.settle();
  }

  async select(action, group = C.default_action_group, zoomAttempt = C.initial_zoom_attempt) {
    await this.jump(action.start_time_unix_nanos);
    const titles = group === 'Messages' ? ['Messages:', '消息：'] : ['Network / protocol:', '网络 / 协议：'];
    const canvases = this.page.locator(titles.map(title => `.flame-track-canvas[aria-label^="${title}"]`).join(','));
    await canvases.first().waitFor();
    const viewport = await this.viewport();
    const attempts = [];
    for (const canvas of await canvases.all()) {
      await canvas.scrollIntoViewIfNeeded();
      await this.settle();
      const box = await canvas.boundingBox();
      const offset = Number(BigInt(action.start_time_unix_nanos) - viewport.origin) / C.nanoseconds_per_ms;
      const x = Math.min(box.width - 1, Math.max(1, (offset - viewport.startMs) / viewport.spanMs * box.width + C.canvas_hit_offset_x));
      for (let y = C.canvas_first_row_y; y < box.height; y += C.canvas_row_height) {
        await canvas.hover({ position: { x, y } });
        const title = await canvas.getAttribute('title');
        attempts.push({ label: await canvas.getAttribute('aria-label'), x, y, title,
          width: box.width, viewport: { ...viewport, origin: String(viewport.origin) }, time: action.start_time_unix_nanos });
        if (!title || /activities|活动|zoom in|放大/i.test(title)) continue;
        const response = this.page.waitForResponse(response => new URL(response.url()).pathname
          === `/api/traces/${encodeURIComponent(this.trace)}/actions/${encodeURIComponent(action.id)}`);
        await canvas.click({ position: { x, y } });
        assert.equal((await response).status(), C.http_ok);
        await this.page.locator('.detail-panel').waitFor();
        await this.settle();
        return { title, x, y };
      }
    }
    if (zoomAttempt < C.max_zoom_attempts && attempts.some(attempt => /activities share|Zoom in|活动|放大/i.test(attempt.title ?? ''))) {
      await this.page.locator('.flame-zoom-controls button').last().click();
      await this.settle();
      return this.select(action, group, zoomAttempt + 1);
    }
    throw new Error(`No canvas hit for actual action ${action.id}: ${JSON.stringify(attempts)}`);
  }
}
