import { EVENT_COLUMNS, eventMatchesQuery, eventRow, windowedTableView } from '../../tableModel';

export function project({ traceDetail, query = '', rowLimit = 0 }) {
  return windowedTableView(EVENT_COLUMNS, traceDetail?.events, eventRow, 'No file events', {
    query,
    rowLimit,
    include: eventHasFileDomain,
    matches: eventMatchesQuery,
  });
}

function eventHasFileDomain(event) {
  return event.domain?.toLowerCase().includes('file');
}
