import { chip, compactChips, compactRows, firstPresent } from '../detail/insight.js';

export function buildCommandDetailInsight(detail) {
  const action = detail?.raw ?? null;
  const attrs = action?.attributes ?? {};
  if (!action || !commandSignal(action.kind)) {
    return null;
  }

  const commandLine = firstPresent(
    attrs['command.line'],
    attrs.command_line,
    attrs['agent.child.command_line'],
    action.kind === 'command.invocation' ? action.title : null,
  );
  const executable = firstPresent(
    attrs['process.executable'],
    attrs.executable,
    attrs['agent.child.executable'],
    action.kind === 'process.exec' ? action.title : null,
  );
  const exitCode = firstPresent(attrs['command.exit_code'], attrs['process.exit_code']);
  const failureKind = firstPresent(attrs['command.failure.kind'], attrs['process.failure.kind']);
  const failureSummary = firstPresent(
    attrs['command.failure.summary'],
    attrs['process.failure.summary'],
    exitCode && String(exitCode) !== '0' ? `exit code ${exitCode}` : null,
  );
  const blocks = compactBlocks([
    failureBlock({ status: action.status, exitCode, failureKind, failureSummary }),
    commandBlock({
      commandLine,
      executable,
      cwd: attrs.cwd,
      argvCount: attrs.argv_count,
      argsTruncated: attrs.args_truncated,
    }),
  ]);
  if (!blocks.length && !commandLine && !executable) {
    return null;
  }
  return {
    instanceId: action.id,
    kind: action.kind,
    heading: action.kind === 'process.exec' ? 'Process Exec' : 'Command',
    chips: compactChips([
      chip('status', action.status),
      chip('exit', exitCode),
      chip('pid', action.process?.pid),
      chip('cwd', attrs.cwd),
    ]),
    blocks,
  };
}

function commandSignal(kind) {
  return kind === 'command.invocation' || kind === 'process.exec';
}

function failureBlock({ status, exitCode, failureKind, failureSummary }) {
  if (!failureSummary && !failureKind && !exitCode && status !== 'error') {
    return null;
  }
  return {
    id: 'command-failure',
    tone: status === 'error' ? 'status' : 'context',
    label: status === 'error' ? 'Failure' : 'Exit',
    title: failureSummary ?? (exitCode ? `exit code ${exitCode}` : status),
    rows: compactRows({
      exit_code: exitCode,
      kind: failureKind,
      status,
    }),
  };
}

function commandBlock({ commandLine, executable, cwd, argvCount, argsTruncated }) {
  if (!commandLine && !executable && !cwd) {
    return null;
  }
  return {
    id: 'command-line',
    tone: 'context',
    label: 'Command',
    title: commandLine ?? executable ?? 'process command',
    rows: compactRows({
      executable,
      cwd,
      argv_count: argvCount,
      args_truncated: argsTruncated,
    }),
    text: commandLine,
  };
}

function compactBlocks(blocks) {
  return blocks.filter(Boolean);
}
