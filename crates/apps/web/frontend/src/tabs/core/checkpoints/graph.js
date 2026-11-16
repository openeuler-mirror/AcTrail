/** The checkpoint path after applying successful restores in execution order. */
import { operationStatus } from './model.js';

/**
 * Each saved checkpoint remembers its path. A successful restore returns to that path, so
 * 0,1,2,3 restored to 1 displays only 0,1. Full records remain available for ActionTree links.
 * Each restore stays on its target's continuation; folded paths keep their own restore history.
 * Pending and failed restores leave the path intact.
 */
export function historyGraph(listing) {
  const checkpoints = new Map(
    (listing?.checkpoints ?? []).map((checkpoint) => [checkpoint.checkpoint_id, checkpoint]),
  );
  const time = (operation) => operation.finished_at ?? operation.started_at ?? operation.requested_at;
  const operations = [...(listing?.operations ?? [])]
    .sort((left, right) => time(left) - time(right) || left.operation_id - right.operation_id);
  const records = operations.map((operation) => {
    const restore = operation.kind === 'restore';
    const checkpoint = checkpoints.get(restore ? operation.target_checkpoint : operation.created_checkpoint) ?? null;
    return {
      key: `operation-${operation.operation_id}`,
      type: restore ? 'restore' : checkpoint ? 'checkpoint' : 'save',
      operation,
      checkpoint,
    };
  });
  const savedPaths = new Map();
  let tip = null;
  for (const row of records) {
    if (row.type === 'restore' && operationStatus(row.operation) === 'succeeded') {
      let target = savedPaths.get(row.operation.target_checkpoint);
      for (let node = tip; node; node = node.previous) {
        if (node.row.key === target.row.key) {
          target = node;
          break;
        }
      }
      // A restore copies its target node, so the same checkpoint can sit on both paths as
      // different objects; keys identify it.
      const targetKeys = new Set();
      for (let node = target; node; node = node.previous) targetKeys.add(node.row.key);
      const discarded = [];
      for (let node = tip; node && !targetKeys.has(node.row.key); node = node.previous) discarded.push(node.row);
      const restore = { ...row, discarded: discarded.reverse() };
      tip = {
        row: { ...target.row, restores: [...(target.row.restores ?? []).filter((previous) => previous.discarded.length > 0), restore] },
        previous: target.previous,
      };
    } else {
      tip = { row, previous: tip };
      if (row.type === 'checkpoint') savedPaths.set(row.checkpoint.checkpoint_id, tip);
    }
  }
  const rows = [];
  for (let node = tip; node; node = node.previous) rows.push(node.row);
  return { rows: rows.reverse(), records };
}

/** Open the end of the displayed path, or its pending/failed operation. */
export function initialRow(rows) {
  return rows.at(-1) ?? null;
}
