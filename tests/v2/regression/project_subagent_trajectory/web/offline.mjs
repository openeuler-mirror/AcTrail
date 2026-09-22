import { pathToFileURL } from 'node:url';
import { resolve } from 'node:path';
import { get } from 'node:http';

class RecordedWebCorrelation {
  constructor(repo, baseUrl, traceId, maxBytes, timeoutSeconds) {
    this.repo = repo;
    this.baseUrl = baseUrl;
    this.traceId = traceId;
    this.maxBytes = Number(maxBytes);
    this.signal = AbortSignal.timeout(Number(timeoutSeconds) * 1000);
  }

  async read(path, { signal = this.signal } = {}) {
    return new Promise((resolve, reject) => {
      const request = get(new URL(path, this.baseUrl), { signal, agent: false }, (response) => {
        response.on('error', reject);
        if (response.statusCode !== 200) {
          response.resume();
          reject(new Error(`Web returned HTTP ${response.statusCode} for ${path}`));
          return;
        }
        const chunks = [];
        response.on('data', (chunk) => chunks.push(chunk));
        response.on('end', () => {
          try {
            resolve(JSON.parse(Buffer.concat(chunks).toString('utf8')));
          } catch (error) {
            reject(error);
          }
        });
      });
      request.on('error', reject);
    });
  }

  async run() {
    const modulePath = resolve(this.repo,
      'crates/apps/web/frontend/src/tabs/core/llm-trajectory/offline-correlation.js');
    const { OfflineAgentCorrelation } = await import(pathToFileURL(modulePath));
    const prefix = `/api/traces/${encodeURIComponent(this.traceId)}`;
    const graph = await this.read(`${prefix}/llm-trajectories`);
    const correlation = new OfflineAgentCorrelation({
      readWaterfall: (_trace, options) => this.read(`${prefix}/waterfall`, options),
      readActionDetail: (_trace, id, options) => this.read(`${prefix}/actions/${encodeURIComponent(id)}`, options),
      readRequestContent: (_trace, id, options) => this.read(
        `${prefix}/actions/${encodeURIComponent(id)}/content/llm-request?max_bytes=${options.maxBytes}`, options),
    }, { maxBytes: this.maxBytes });
    const result = await correlation.derive(this.traceId, graph, this.signal);
    process.stdout.write(`${JSON.stringify(result)}\n`);
  }
}

await new RecordedWebCorrelation(...process.argv.slice(2)).run();
