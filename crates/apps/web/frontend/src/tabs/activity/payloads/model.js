import { compactRows, formatTime, row, valuesMatchQuery, windowedTableView } from '../../tableModel';

const COLUMNS = Object.freeze([
  { key: 'id', label: 'ID' },
  { key: 'time', label: 'Time' },
  { key: 'pid', label: 'PID' },
  { key: 'direction', label: 'Direction' },
  { key: 'source', label: 'Source' },
  { key: 'size', label: 'Size' },
]);

export function project({ traceDetail, query = '', rowLimit = 0 }) {
  return windowedTableView(COLUMNS, traceDetail?.payloads, payloadRow, 'No payloads', {
    query,
    rowLimit,
    matches: payloadMatchesQuery,
  });
}

function payloadRow(payload) {
  return row(
    `payload:${payload.id}`,
    {
      id: payload.display_id ?? payload.id,
      time: formatTime(payload.observed_at),
      pid: payload.pid,
      direction: payload.direction,
      source: payload.source,
      size: `${payload.captured_size ?? ''}/${payload.original_size ?? ''}`,
    },
    {
      title: payload.display_id ?? `payload ${payload.id}`,
      kind: 'payload',
      rows: compactRows({
        pid: payload.pid,
        direction: payload.direction,
        source: payload.source,
        protocol: payload.protocol_hint,
        observed: payload.observed_at,
      }),
      raw: payload,
      payloadId: payload.id,
    },
  );
}

function payloadMatchesQuery(payload, query) {
  return valuesMatchQuery(
    [
      payload.display_id ?? payload.id,
      formatTime(payload.observed_at),
      payload.pid,
      payload.direction,
      payload.source,
      payload.protocol_hint,
      payload.captured_size,
      payload.original_size,
    ],
    query,
  );
}
