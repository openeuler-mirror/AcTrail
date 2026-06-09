export const EVENT_COLUMNS = Object.freeze([
  { key: 'id', label: 'ID' },
  { key: 'time', label: 'Time' },
  { key: 'pid', label: 'PID' },
  { key: 'operation', label: 'Operation' },
  { key: 'summary', label: 'Summary' },
]);

export function tableView(columns, rows, emptyLabel, options = {}) {
  return { columns, rows, emptyLabel, ...options };
}

export function windowedTableView(columns, items, rowMapper, emptyLabel, options = {}) {
  const projection = projectWindowedRows(items, rowMapper, options);
  return tableView(columns, projection.rows, emptyLabel, projection);
}

export function projectWindowedRows(items = [], rowMapper, options = {}) {
  const source = items ?? [];
  const rowLimit = positiveInteger(options.rowLimit);
  const query = normalizeTableQuery(options.query);
  if (!query && !options.include) {
    const visibleItems = rowLimit ? source.slice(0, rowLimit) : source;
    return {
      rows: visibleItems.map(rowMapper),
      totalRows: source.length,
      queryApplied: true,
    };
  }

  const rows = [];
  let totalRows = 0;
  for (const item of source) {
    if (options.include && !options.include(item)) {
      continue;
    }
    if (query && !itemMatchesQuery(item, rowMapper, query, options.matches)) {
      continue;
    }
    totalRows += 1;
    if (!rowLimit || rows.length < rowLimit) {
      rows.push(rowMapper(item));
    }
  }
  return { rows, totalRows, queryApplied: true };
}

export function filterTableRows(rows, query) {
  return filterRows(rows, normalizeTableQuery(query));
}

export function row(id, cells, detail) {
  return { id, cells, detail };
}

export function eventDetail(event) {
  return {
    title: event.summary || event.operation || event.display_id,
    kind: event.domain,
    rows: compactRows({
      id: event.display_id ?? event.id,
      pid: event.pid,
      operation: event.operation,
      observed: event.observed_at,
    }),
    attributes: event.metadata,
    raw: event,
  };
}

export function eventRows(events = []) {
  return events.map((event) =>
    eventRow(event),
  );
}

export function eventRow(event) {
  return row(
    `event:${event.id}`,
    {
      id: event.display_id ?? event.id,
      time: formatTime(event.observed_at),
      pid: event.pid,
      operation: event.operation,
      summary: event.summary,
    },
    eventDetail(event),
  );
}

export function eventMatchesQuery(event, query) {
  return valuesMatchQuery(
    [
      event.display_id ?? event.id,
      formatTime(event.observed_at),
      event.pid,
      event.operation,
      event.summary,
      event.domain,
    ],
    query,
  );
}

export function formatTime(value) {
  if (!value) {
    return '';
  }
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

export function compactRows(rows) {
  return Object.fromEntries(
    Object.entries(rows).filter(([, value]) => value !== undefined && value !== null && value !== ''),
  );
}

export function normalizeTableQuery(value) {
  return String(value ?? '').trim().toLowerCase();
}

export function positiveInteger(value) {
  const number = Number(value);
  return Number.isInteger(number) && number > 0 ? number : 0;
}

export function valuesMatchQuery(values, query) {
  return values.some((value) => String(value ?? '').toLowerCase().includes(query));
}

function itemMatchesQuery(item, rowMapper, query, matches) {
  if (matches) {
    return matches(item, query);
  }
  return rowMatchesQuery(rowMapper(item), query);
}

function filterRows(rows, query) {
  if (!query) {
    return rows;
  }
  return rows.filter((item) =>
    rowMatchesQuery(item, query),
  );
}

function rowMatchesQuery(item, query) {
  return Object.values(item.cells).some((cell) => cellText(cell).toLowerCase().includes(query));
}

function cellText(cell) {
  if (cell && typeof cell === 'object' && Object.prototype.hasOwnProperty.call(cell, 'text')) {
    return String(cell.text ?? '');
  }
  return String(cell ?? '');
}
