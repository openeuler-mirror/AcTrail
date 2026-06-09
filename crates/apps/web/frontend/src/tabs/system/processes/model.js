import { compactRows, formatTime, row, valuesMatchQuery, windowedTableView } from '../../tableModel';

const COLUMNS = Object.freeze([
  { key: 'pid', label: 'PID' },
  { key: 'parent', label: 'Parent' },
  { key: 'state', label: 'State' },
  { key: 'exit', label: 'Exit' },
  { key: 'observed', label: 'Observed' },
]);

export function project({ traceDetail, query = '', rowLimit = 0 }) {
  return windowedTableView(COLUMNS, traceDetail?.processes, processRow, 'No processes', {
    query,
    rowLimit,
    matches: processMatchesQuery,
  });
}

function processRow(process) {
  return row(
    `process:${process.pid}:${process.identity?.generation ?? ''}`,
    {
      pid: process.pid,
      parent: process.parent_pid,
      state: process.state,
      exit: process.exit_code,
      observed: formatTime(process.observed_at),
    },
    {
      title: `pid ${process.pid}`,
      kind: 'process',
      rows: compactRows({
        pid: process.pid,
        parent_pid: process.parent_pid,
        state: process.state,
        exit_code: process.exit_code,
        observed: process.observed_at,
        exit_observed: process.exit_observed_at,
      }),
      raw: process,
    },
  );
}

function processMatchesQuery(process, query) {
  return valuesMatchQuery(
    [
      process.pid,
      process.parent_pid,
      process.state,
      process.exit_code,
      formatTime(process.observed_at),
      process.identity?.generation,
    ],
    query,
  );
}
