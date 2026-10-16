// Verify real indexed range reads and continuation under a one-candidate budget.
import fs from 'node:fs/promises';
import assert from 'node:assert/strict';
const [base, traceId, out] = process.argv.slice(2);
const fixture = JSON.parse(await fs.readFile(`${out}/small-budget-case.json`, 'utf8'));
const query = new URLSearchParams({ limit: '1', kinds: fixture.kind, from: fixture.from });
const path = `${base}/api/traces/${traceId}/actions/timeline`;
const ids = [], pages = [], cursors = new Set();
for (let index = 0; index < fixture.ids.length + 3; index++) {
  const response = await fetch(`${path}?${query}`);
  assert.equal(response.status, 200);
  const result = await response.json();
  assert(result.scanned <= 1 && result.actions.length <= 1);
  assert(result.actions.every(action => BigInt(action.start_time_unix_nanos) >= BigInt(fixture.from)));
  ids.push(...result.actions.map(action => action.id));
  pages.push(result);
  if (!result.has_more) break;
  assert(result.next_cursor && !cursors.has(result.next_cursor), 'bounded cursor did not advance');
  cursors.add(result.next_cursor);
  query.set('cursor', result.next_cursor);
}
assert.deepEqual(ids.sort(), fixture.ids, 'one-candidate range query lost or repeated real actions');
await fs.writeFile(`${out}/small-budget.json`, JSON.stringify({ status: 'passed', pages }, null, 2));
