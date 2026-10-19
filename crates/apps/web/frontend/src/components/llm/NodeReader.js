// Limit concurrent node reads within one request view and cancel them on disposal.
import { readActionLlmRequestContentNode } from '../../api';

export class NodeReader {
  constructor(traceId, actionId, concurrency) {
    if (!Number.isSafeInteger(concurrency) || concurrency < 1) throw new Error('Node read concurrency must be positive');
    this.traceId = traceId;
    this.actionId = actionId;
    this.concurrency = concurrency;
    this.active = 0;
    this.queue = [];
    this.controller = new AbortController();
  }

  read(query) {
    return new Promise((resolve, reject) => {
      if (this.controller.signal.aborted) { reject(new DOMException('Cancelled', 'AbortError')); return; }
      this.queue.push({ query, resolve, reject });
      this.drain();
    });
  }

  drain() {
    while (this.active < this.concurrency && this.queue.length) {
      const item = this.queue.shift();
      this.active += 1;
      readActionLlmRequestContentNode(this.traceId, this.actionId, { ...item.query, signal: this.controller.signal })
        .then(response => {
          if (!response.content?.node) throw new Error('Request content is unavailable');
          item.resolve(response.content.node);
        }).catch(item.reject).finally(() => { this.active -= 1; this.drain(); });
    }
  }

  close() {
    this.controller.abort();
    for (const item of this.queue.splice(0)) item.reject(new DOMException('Cancelled', 'AbortError'));
  }
}
