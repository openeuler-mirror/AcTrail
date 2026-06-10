import { compactRows, formatTime, row, valuesMatchQuery, windowedTableView } from '../../tableModel';
import { semanticActionLabel, semanticActionTarget } from '../../actionLabels';

const COLUMNS = Object.freeze([
  { key: 'time', label: 'Time' },
  { key: 'duration', label: 'Duration' },
  { key: 'pid', label: 'PID' },
  { key: 'kind', label: 'Kind' },
  { key: 'status', label: 'Status' },
  { key: 'title', label: 'Title' },
]);

export function project({ tabData, query = '', rowLimit = 0 }) {
  return windowedTableView(COLUMNS, tabData?.actions, commandRow, 'No commands', {
    query,
    rowLimit,
    include: actionIsCommand,
    matches: commandMatchesQuery,
  });
}

function commandRow(action) {
  const label = semanticActionLabel(action);
  const target = semanticActionTarget(action);
  return row(
    `command:${action.id}`,
    {
      time: formatTime(action.start_time),
      duration: action.duration,
      pid: action.process?.pid,
      kind: label,
      status: action.status,
      title: target || action.title || label,
    },
    {
      title: target || action.title || label,
      kind: label,
      rows: compactRows({
        semantic_label: label,
        raw_action_kind: action.kind,
        target,
        status: action.status,
        completeness: action.completeness,
        pid: action.process?.pid,
        started: action.start_time,
        ended: action.end_time,
        duration: action.duration,
      }),
      attributes: action.attributes,
      raw: action,
    },
  );
}

function actionIsCommand(action) {
  return action.kind === 'command.invocation' || action.kind === 'process.exec';
}

function commandMatchesQuery(action, query) {
  return valuesMatchQuery(
    [
      formatTime(action.start_time),
      action.duration,
      action.process?.pid,
      action.kind,
      semanticActionLabel(action),
      semanticActionTarget(action),
      action.status,
      action.title,
      action.completeness,
    ],
    query,
  );
}
