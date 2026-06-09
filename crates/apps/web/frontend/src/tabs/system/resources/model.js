import { EVENT_COLUMNS, eventMatchesQuery, eventRow, windowedTableView } from '../../tableModel';

export function project({ traceDetail, query = '', rowLimit = 0 }) {
  return windowedTableView(EVENT_COLUMNS, traceDetail?.events, eventRow, 'No resource events', {
    query,
    rowLimit,
    include: eventHasResourceDomain,
    matches: eventMatchesQuery,
  });
}

function eventHasResourceDomain(event) {
  return event.domain?.toLowerCase().includes('resource');
}
