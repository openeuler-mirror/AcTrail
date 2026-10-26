import test from 'node:test';
import assert from 'node:assert/strict';

import { buildTrajectoryLayout, nodeLabel, trajectoryColor } from './model.js';

test('trajectory layout assigns stable lanes and connects append and fork edges', () => {
  const graph = {
    nodes: [
      request('b', 'root-a', '20'),
      request('a', 'root-a', '10'),
      request('c', 'root-c', '30'),
    ],
    edges: [
      { source: 'a', target: 'b', kind: 'append' },
      { source: 'b', target: 'c', kind: 'fork' },
    ],
  };

  const layout = buildTrajectoryLayout(graph);

  assert.deepEqual(layout.nodes.map((node) => node.id), ['a', 'b', 'c']);
  assert.equal(layout.nodes[0].lane, layout.nodes[1].lane);
  assert.notEqual(layout.nodes[1].lane, layout.nodes[2].lane);
  assert.match(layout.edges[0].path, /^M /);
  assert.equal(trajectoryColor('root-a'), trajectoryColor('root-a'));
  assert.deepEqual(
    layout.trajectories.map(({ label, lane }) => ({ label, lane })),
    [
      { label: 'T1', lane: 0 },
      { label: 'T2', lane: 1 },
    ],
  );
});

test('trajectory layout reuses lanes for non-overlapping unrelated contexts', () => {
  const graph = {
    nodes: [
      request('a', 'first', '10', 0),
      request('b', 'first', '20', 1),
      request('c', 'second', '30', 0),
      request('d', 'second', '40', 1),
    ],
    edges: [
      { source: 'a', target: 'b', kind: 'append' },
      { source: 'c', target: 'd', kind: 'append' },
    ],
  };

  const layout = buildTrajectoryLayout(graph);

  assert.equal(layout.laneCount, 1);
  assert.deepEqual(layout.trajectories.map(({ lane }) => lane), [0, 0]);
  assert.deepEqual(
    layout.trajectories.map(({ reusedLane }) => reusedLane),
    [false, true],
  );
  assert.notEqual(layout.trajectories[0].color, layout.trajectories[1].color);
});

test('trajectory layout keeps overlapping contexts in separate lanes', () => {
  const graph = {
    nodes: [
      request('a', 'first', '10', 0),
      request('c', 'second', '20', 0),
      request('b', 'first', '30', 1),
    ],
    edges: [{ source: 'a', target: 'b', kind: 'append' }],
  };

  const layout = buildTrajectoryLayout(graph);

  assert.equal(layout.laneCount, 2);
  assert.notEqual(layout.trajectories[0].lane, layout.trajectories[1].lane);
});

test('node labels use readable hierarchy and preserve missing counts', () => {
  assert.deepEqual(
    nodeLabel(
      {
        model: 'demo',
        trajectory_position: 0,
        block_count: null,
        user_message_count: 0,
        tool_result_count: null,
      },
      'T2',
    ),
    {
      title: 'T2 · Step 1 · demo',
      metadata: '— blocks · 0 user messages · — tool results',
    },
  );
});

test('node labels show count growth from the previous append', () => {
  assert.deepEqual(
    nodeLabel(
      {
        model: 'demo',
        trajectory_position: 2,
        block_count: 18,
        user_message_count: 2,
        tool_result_count: 1,
      },
      'T1',
      {
        block_count: 16,
        user_message_count: 1,
        tool_result_count: 1,
      },
    ),
    {
      title: 'T1 · Step 3 · demo',
      metadata: '18 blocks (+2) · 2 user messages (+1) · 1 tool result',
    },
  );
});

function request(id, trajectoryId, nanos, position = 0) {
  return {
    id,
    trajectory_id: trajectoryId,
    trajectory_position: position,
    start_time_unix_nanos: nanos,
    model: 'demo',
    block_count: 1,
    user_message_count: 1,
    tool_result_count: 0,
  };
}
