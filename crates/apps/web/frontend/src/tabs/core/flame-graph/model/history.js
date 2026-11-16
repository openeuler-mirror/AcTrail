// Derive display classification and call numbering from the overview index.
import { activityFromAction } from './presentation.js';

const MODEL_MESSAGE_KINDS = new Set(['llm.request', 'llm.response']);

export class FlameGraphHistory {
  constructor() {
    this.index = null;
    this.traceKey = null;
    this.classifications = new Map();
    this.callOrdinalById = new Map();
    this.ordinalRevision = null;
    this.completed = new WeakMap();
    this.language = null;
  }

  sync(index, traceKey, language) {
    if (index !== this.index || traceKey !== this.traceKey || !index?.records.size) {
      this.classifications.clear();
      this.callOrdinalById.clear();
      this.ordinalRevision = null;
      this.completed = new WeakMap();
    }
    if (language !== this.language) this.completed = new WeakMap();
    this.index = index;
    this.traceKey = traceKey;
    this.language = language;
  }

  get size() {
    return this.index?.records.size ?? 0;
  }

  ordinals() {
    // An incomplete scan cannot supply a trace-wide rank for a newly seen call.
    if (!this.index || this.index.loading.value || this.index.error.value || !this.index.initialized) return this.callOrdinalById;
    const revision = this.index.revision.value;
    if (revision === this.ordinalRevision) return this.callOrdinalById;
    const calls = [];
    for (const record of this.index.records.values()) {
      if (record.kind === 'llm.call') calls.push(record);
    }
    calls.sort((left, right) => {
      if (left.startNanos < right.startNanos) return -1;
      if (left.startNanos > right.startNanos) return 1;
      return String(left.id).localeCompare(String(right.id));
    });
    if (calls.length !== this.callOrdinalById.size
      || calls.some((call, index) => this.callOrdinalById.get(call.id) !== index + 1)) {
      this.callOrdinalById = new Map(calls.map((call, index) => [call.id, index + 1]));
    }
    this.ordinalRevision = revision;
    return this.callOrdinalById;
  }

  merge(activities, window, toolDisplay, t) {
    if (!this.size) return;
    const residentIds = new Set();
    for (const activity of activities) {
      residentIds.add(activity.id);
      if (!MODEL_MESSAGE_KINDS.has(activity.kind) && this.index.records.has(activity.id)) {
        const known = this.classifications.get(activity.id);
        if (!known || known.layer !== activity.layer || known.group !== activity.group
          || known.backgroundKind !== activity.backgroundKind) {
          this.classifications.set(activity.id, {
            layer: activity.layer,
            group: activity.group,
            backgroundKind: activity.backgroundKind,
          });
        }
      }
    }
    for (const record of this.index.records.values()) {
      if (residentIds.has(record.id) || record.kind === 'agent.context_compaction'
        || MODEL_MESSAGE_KINDS.has(record.kind)) continue;
      const known = this.classifications.get(record.id);
      // Timing-only summaries cannot provide command text or a verified parent.
      // Keep them in the overview; Commands uses the detailed window actions.
      if (known?.group === 'commands' || (!known && record.kind === 'command.invocation')) continue;
      activities.push(this.activity(record, window, toolDisplay, t));
    }
  }

  activity(record, window, toolDisplay, t) {
    const known = this.classifications.get(record.id);
    const ordinal = this.callOrdinalById.get(record.id) ?? null;
    const cached = this.completed.get(record);
    if (cached && cached.classification === known && cached.ordinal === ordinal
      && cached.origin === window.startNanos) return cached.activity;
    const layer = known?.layer
      ?? (record.kind.startsWith('llm.') || record.kind === 'agent.invocation' ? 'agent' : 'harness');
    const action = {
      id: record.id,
      kind: record.kind,
      status: record.status,
      start_time_unix_nanos: String(record.startNanos),
      end_time_unix_nanos: record.endNanos === null ? null : String(record.endNanos),
    };
    const activity = activityFromAction(
      action, window, layer, null, known?.backgroundKind ?? null,
      ordinal, null, null, toolDisplay, t,
    );
    activity.group = known?.group ?? activity.group;
    if (record.kind !== 'llm.call') {
      activity.label = `${record.kind} · ${record.id}`;
    }
    activity.searchText = `${activity.label} ${record.kind} ${record.id} ${record.status}`.toLowerCase();
    activity.modelMessage = false;
    activity.scopeIncomplete = true;
    activity.lightweight = true;
    activity.aggregate = false;
    // Completed activities do not consume traceSpanMs; only running ones grow.
    if (record.endNanos !== null) this.completed.set(record, {
      classification: known, ordinal, origin: window.startNanos, activity,
    });
    return activity;
  }
}
