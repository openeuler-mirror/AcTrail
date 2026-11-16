// Verify real resource responses and their canvases on the shared flame-graph axis.
import assert from 'node:assert/strict';
import { TimelineBrowser } from './timeline.mjs';
import { C } from '../constants.mjs';

export class ResourceAcceptance extends TimelineBrowser {
  constructor(page, base, trace, report) {
    super(page, base, trace);
    this.report = report;
    this.resourcePath = `/api/traces/${encodeURIComponent(trace)}/resource-timeline`;
    this.resourceResponses = [];
    this.resourcePending = new Set();
    page.on('response', response => {
      if (new URL(response.url()).pathname !== this.resourcePath) return;
      const pending = response.json().then(data => this.resourceResponses.push({
        url: response.url(), status: response.status(), data,
      })).catch(error => { this.resourceError = String(error); })
        .finally(() => this.resourcePending.delete(pending));
      this.resourcePending.add(pending);
    });
  }

  setMetadata(metadata) {
    super.setMetadata({ ...metadata, timeline_bounds: metadata.timeline_bounds ?? {} });
  }

  async verify({ empty = false } = {}) {
    const layer = this.page.locator('.layer-res');
    const toggle = layer.locator('.flame-layer-header');
    await layer.waitFor();
    if (await toggle.getAttribute('aria-expanded') !== 'true') await toggle.click();
    await this.settleResources();
    const initial = await this.checkState(empty);
    if (!empty) {
      assert(initial.spanMs > 1, 'resource capture is too short to exercise zoom');
      await this.page.locator('.flame-zoom-controls button').last().click();
      await this.settleResources();
      const zoomed = await this.checkState(false);
      assert(zoomed.spanMs < initial.spanMs, 'resource zoom did not change viewport');
      for (const fraction of C.pan_fractions.slice(0, 2)) {
        const start = BigInt(this.bounds.start_time_unix_nanos);
        const span = BigInt(this.bounds.end_time_unix_nanos) - start;
        await this.jump(String(start + BigInt(Math.round(Number(span) * fraction))));
        await this.settleResources();
        await this.checkState(false);
      }
    }
    await toggle.click();
    assert.equal(await toggle.getAttribute('aria-expanded'), 'false');
    assert.equal(await layer.locator('canvas').count(), 0, 'collapsed resource area retained canvases');
    await toggle.click();
    await this.settleResources();
    await this.checkState(empty);
    await this.page.locator('.flame-zoom-controls button').nth(1).click();
    await this.settleResources();
    this.report.resources = { status: 'passed', empty, requests: this.resourceResponses.length };
  }

  async settleResources() {
    await this.settle();
    await Promise.all([...this.resourcePending]);
    assert(!this.resourceError, this.resourceError);
  }

  async checkState(empty) {
    const layer = this.page.locator('.layer-res');
    assert.equal(await layer.locator('.res-error').count(), 0, 'resource area displayed a load error');
    assert.equal(await layer.locator('.res-group-row').count(), 2, 'resource host/workload groups are missing');
    const response = this.resourceResponses.at(-1);
    assert(response, 'resource area never requested a real timeline');
    assert.equal(response.status, 200);
    const { data } = response;
    const query = new URL(response.url).searchParams;
    assert.equal(data.trace_id, Number(this.trace));
    assert.equal(data.from_ms, Number(query.get('from_ms')));
    assert.equal(data.to_ms, Number(query.get('to_ms')));
    assert(Number(query.get('max_points')) > 0 && Number(query.get('max_points')) <= 2000);
    const viewport = await this.viewport();
    const from = Number(viewport.origin / 1_000_000n) + viewport.startMs;
    const to = from + viewport.spanMs;
    assert(data.from_ms <= Math.floor(from) + data.bucket_ms
      && data.to_ms >= Math.ceil(to) - data.bucket_ms, 'resource query no longer covers the flame viewport');
    if (empty) {
      for (const group of ['host', 'workload']) {
        assert.equal(data[group].available, false);
        assert.deepEqual(data[group].points, []);
        assert(data[group].reasons.includes('no_samples'));
      }
      assert.equal(await layer.locator('.res-track-canvas').count(), 0);
      assert(await layer.locator('.res-reason').count() >= 2, 'empty groups have no explanation');
    } else {
      assert(data.host.available && data.workload.available, 'real capture lacks host/workload resource samples');
      assert(data.host.points.length && data.workload.points.length);
      assert(await layer.locator(`.source-${data.workload.source}`).count(), 'resource source label is missing');
      const activityTrack = this.page.locator('.flame-layer:not(.layer-res) .fg-time-track').first();
      let plot;
      if (await activityTrack.count()) {
        plot = await activityTrack.boundingBox();
      } else {
        // Empty activity layers still define the common card content area. The header axis
        // sits outside those cards and includes their margins, so compare plot content here.
        const card = await this.page.locator('.flame-layer:not(.layer-res)').first().evaluate(element => ({
          x: element.getBoundingClientRect().x + element.clientLeft, width: element.clientWidth,
        }));
        const gutter = await this.page.locator('.flame-axis .fg-gutter').boundingBox();
        plot = { x: card.x + gutter.width, width: card.width - gutter.width };
      }
      const canvases = layer.locator('.res-track-canvas');
      assert(await canvases.count() >= 2, 'resource tracks did not mount');
      for (const canvas of await canvases.all()) {
        const box = await canvas.boundingBox();
        assert(box && box.height > 1 && Math.abs(box.x - plot.x) <= 2
          && Math.abs(box.width - plot.width) <= 2,
        `resource canvas does not align with the flame plot: ${JSON.stringify({ plot, box, label: await canvas.getAttribute('aria-label') })}`);
      }
      const painted = await canvases.evaluateAll(elements => elements.filter(canvas => {
        const pixels = canvas.getContext('2d').getImageData(0, 0, canvas.width, canvas.height).data;
        // The baseline/grid is translucent. An opaque colored stroke proves data was drawn.
        for (let index = 0; index < pixels.length; index += 4) {
          if (pixels[index + 3] > 120 && Math.max(...pixels.slice(index, index + 3))
            - Math.min(...pixels.slice(index, index + 3)) > 15) return true;
        }
        return false;
      }).length);
      assert(painted > 0, 'resource canvases contain no visible data strokes');
    }
    return viewport;
  }
}
