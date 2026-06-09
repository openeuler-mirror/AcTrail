import { compactRows, formatTime, row, valuesMatchQuery, windowedTableView } from '../../tableModel';

const COLUMNS = Object.freeze([
  { key: 'time', label: 'Time' },
  { key: 'kind', label: 'Kind' },
  { key: 'lane', label: 'Lane' },
  { key: 'pid', label: 'PID' },
  { key: 'title', label: 'Title' },
  { key: 'summary', label: 'Summary' },
]);

export function project({ traceDetail, query = '', rowLimit = 0 }) {
  return windowedTableView(COLUMNS, traceDetail?.timeline, timelineRow, 'No timeline rows', {
    query,
    rowLimit,
    matches: matchesTimelineQuery,
  });
}

function timelineRow(item) {
  return row(
    `timeline:${item.kind}:${item.id}`,
    {
      time: formatTime(item.observed_at),
      kind: item.kind,
      lane: item.lane,
      pid: item.pid,
      title: item.title,
      summary: item.summary,
    },
    {
      title: item.title || item.kind,
      kind: `timeline.${item.kind}`,
      rows: compactRows({
        lane: item.lane,
        pid: item.pid,
        observed: item.observed_at,
      }),
      raw: item,
    },
  );
}

function matchesTimelineQuery(item, query) {
  return valuesMatchQuery(
    [formatTime(item.observed_at), item.kind, item.lane, item.pid, item.title, item.summary],
    query,
  );
}
