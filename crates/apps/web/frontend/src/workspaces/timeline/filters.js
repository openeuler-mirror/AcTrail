// Build bounded timeline queries from the visible action and time filters.
export class TimelineFilters {
  static defaultKinds = ['command.invocation', 'agent.invocation', 'agent.context_compaction', 'llm.call', 'llm.request', 'llm.response', 'http.message'];
  static kinds = [...TimelineFilters.defaultKinds, 'llm.tool_call', 'llm.tool_result',
    'process.exec', 'process.fork_attempt', 'process.exit', 'file.read', 'file.write',
    'file.modify', 'file.tty_io', 'file.bulk_read', 'fs.enumerate',
    'sse.stream', 'sse.event', 'mcp.tool_call', 'mcp.request', 'mcp.response', 'enforcement.decision'];

  constructor(filters) {
    this.reset(filters);
  }

  reset(filters) {
    this.process_id = filters.process_id;
    this.from = filters.from;
    this.to = filters.to;
    this.limit = filters.limit;
    const kinds = String(filters.kinds ?? '').split(',').map((kind) => kind.trim()).filter(Boolean);
    this.selectedKinds = kinds.filter((kind) => TimelineFilters.kinds.includes(kind));
    this.otherKinds = kinds.filter((kind) => !TimelineFilters.kinds.includes(kind)).join(',');
    this.preset = !kinds.length ? 'all'
      : kinds.length === TimelineFilters.defaultKinds.length
        && TimelineFilters.defaultKinds.every((kind) => kinds.includes(kind)) ? 'default' : 'custom';
  }

  query() {
    const kinds = this.preset === 'all' ? [] : this.preset === 'default' ? TimelineFilters.defaultKinds
      : [...this.selectedKinds, ...this.otherKinds.split(',').map((kind) => kind.trim()).filter(Boolean)];
    return { kinds: [...new Set(kinds)].join(','), process_id: this.process_id,
      from: this.from, to: this.to, limit: this.limit };
  }

  localTime(field) {
    if (!this[field]) return '';
    const date = new Date(Number(BigInt(this[field]) / 1_000_000n));
    if (!Number.isFinite(date.getTime())) return '';
    const pad = (value, width = 2) => String(value).padStart(width, '0');
    return `${pad(date.getFullYear(), 4)}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`
      + `T${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}.${pad(date.getMilliseconds(), 3)}`;
  }

  setLocalTime(field, value) {
    if (!value) {
      this[field] = '';
      return;
    }
    const millis = new Date(value).getTime();
    if (Number.isFinite(millis)) this[field] = (BigInt(millis) * 1_000_000n).toString();
  }
}
