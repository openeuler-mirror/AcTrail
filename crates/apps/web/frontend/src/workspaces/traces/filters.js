export class TraceFilters {
  constructor(filters) { this.reset(filters); }

  reset(filters) {
    for (const key of ['name', 'trace_id', 'from', 'to', 'limit']) this[key] = filters[key];
  }

  query() {
    return { name: this.name, trace_id: this.trace_id, from: this.from, to: this.to, limit: this.limit };
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
    if (!value) { this[field] = ''; return; }
    const millis = new Date(value).getTime();
    if (Number.isFinite(millis)) this[field] = (BigInt(millis) * 1_000_000n).toString();
  }
}
