import { EVENT_COLUMNS, eventMatchesQuery, eventRow, windowedTableView } from '../../tableModel';

export function project({ traceDetail, query = '', rowLimit = 0 }) {
  return windowedTableView(EVENT_COLUMNS, traceDetail?.events, eventRow, 'No events', {
    query,
    rowLimit,
    matches: eventMatchesQuery,
  });
}
