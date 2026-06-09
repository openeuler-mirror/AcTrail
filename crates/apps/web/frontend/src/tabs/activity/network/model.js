import { EVENT_COLUMNS, eventMatchesQuery, eventRow, windowedTableView } from '../../tableModel';

export function project({ traceDetail, query = '', rowLimit = 0 }) {
  return windowedTableView(EVENT_COLUMNS, traceDetail?.events, eventRow, 'No network events', {
    query,
    rowLimit,
    include: eventHasNetworkDomain,
    matches: eventMatchesQuery,
  });
}

function eventHasNetworkDomain(event) {
  return event.domain?.toLowerCase().includes('net');
}
