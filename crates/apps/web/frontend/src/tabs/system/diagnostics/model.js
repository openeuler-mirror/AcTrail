import { compactRows, row, valuesMatchQuery, windowedTableView } from '../../tableModel';

const COLUMNS = Object.freeze([
  { key: 'id', label: 'ID' },
  { key: 'severity', label: 'Severity' },
  { key: 'kind', label: 'Kind' },
  { key: 'message', label: 'Message' },
]);

export function project({ traceDetail, query = '', rowLimit = 0 }) {
  return windowedTableView(COLUMNS, traceDetail?.diagnostics, diagnosticRow, 'No diagnostics', {
    query,
    rowLimit,
    matches: diagnosticMatchesQuery,
  });
}

function diagnosticRow(diagnostic) {
  return row(
    `diagnostic:${diagnostic.id}`,
    {
      id: diagnostic.id,
      severity: diagnostic.severity,
      kind: diagnostic.kind,
      message: diagnostic.message,
    },
    {
      title: diagnostic.message,
      kind: diagnostic.kind,
      rows: compactRows({
        severity: diagnostic.severity,
        id: diagnostic.id,
      }),
      attributes: diagnostic.metadata,
      raw: diagnostic,
    },
  );
}

function diagnosticMatchesQuery(diagnostic, query) {
  return valuesMatchQuery([diagnostic.id, diagnostic.severity, diagnostic.kind, diagnostic.message], query);
}
