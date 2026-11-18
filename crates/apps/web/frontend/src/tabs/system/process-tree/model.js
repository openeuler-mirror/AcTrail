import { compactRows, previewText, row, valuesMatchQuery, windowedTableView } from '../../tableModel';

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
    `process-tree:${process.process_id ?? `${process.pid}:${process.generation}`}`,
    {
      process: { text: processLabel(process), indent: process.depth },
      parent: process.parent_pid,
      children: process.children,
      state: process.state,
      generation: process.generation,
    },
    {
      title: process.command_line || `pid ${process.pid ?? "—"}`,
      kind: 'process.tree',
      rows: compactRows({
        pid: process.pid,
        process_id: process.process_id,
        command_line: process.command_line,
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
    [process.command_line, process.pid, process.parent_pid, process.children, process.depth, process.state, process.generation],
    query,
  );
}

function processLabel(process) {
  const pid = `pid ${process.pid ?? "—"}`;
  return process.command_line ? `${pid} · ${previewText(process.command_line)}` : pid;
}
