import { compactRows, row, valuesMatchQuery, windowedTableView } from '../../tableModel';

const COLUMNS = Object.freeze([
  { key: 'process', label: 'Process' },
  { key: 'parent', label: 'Parent' },
  { key: 'children', label: 'Children' },
  { key: 'state', label: 'State' },
  { key: 'generation', label: 'Generation' },
]);

export function project({ traceDetail, query = '', rowLimit = 0 }) {
  return windowedTableView(COLUMNS, traceDetail?.process_tree, processTreeRow, 'No process tree rows', {
    query,
    rowLimit,
    matches: processTreeMatchesQuery,
  });
}

function processTreeRow(process) {
  return row(
    `process-tree:${process.pid}:${process.generation}`,
    {
      process: { text: `pid ${process.pid}`, indent: process.depth },
      parent: process.parent_pid,
      children: process.children,
      state: process.state,
      generation: process.generation,
    },
    {
      title: `pid ${process.pid}`,
      kind: 'process.tree',
      rows: compactRows({
        pid: process.pid,
        parent_pid: process.parent_pid,
        children: process.children,
        depth: process.depth,
        state: process.state,
      }),
      raw: process,
    },
  );
}

function processTreeMatchesQuery(process, query) {
  return valuesMatchQuery(
    [process.pid, process.parent_pid, process.children, process.depth, process.state, process.generation],
    query,
  );
}
