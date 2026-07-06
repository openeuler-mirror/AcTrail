import test from 'node:test';
import assert from 'node:assert/strict';

import { mcpPayloadEvidenceIds, mcpPayloadEvidenceRole } from './payloadEvidence.js';

test('mcpPayloadEvidenceRole does not load payload for protocol request and response nodes', () => {
  assert.equal(mcpPayloadEvidenceRole('mcp.request'), null);
  assert.equal(mcpPayloadEvidenceRole('mcp.response'), null);
});

test('mcpPayloadEvidenceRole keeps stdio payload roles distinct', () => {
  assert.equal(mcpPayloadEvidenceRole('mcp.stdin'), 'mcp.stdin.payload');
  assert.equal(mcpPayloadEvidenceRole('mcp.stdout'), 'mcp.stdout.payload');
});

test('mcpPayloadEvidenceRole keeps remote client payload roles distinct', () => {
  assert.equal(mcpPayloadEvidenceRole('mcp.client_send'), 'mcp.client_send.payload');
  assert.equal(mcpPayloadEvidenceRole('mcp.client_receive'), 'mcp.client_receive.payload');
});

test('mcpPayloadEvidenceRole ignores non-MCP payload details', () => {
  assert.equal(mcpPayloadEvidenceRole('http.message'), null);
});

test('mcpPayloadEvidenceIds returns all matching MCP payload segments in evidence order', () => {
  const action = {
    kind: 'mcp.client_receive',
    evidence: [
      { id: 192, role: 'mcp.client_receive.payload' },
      { id: 196, role: 'mcp.client_receive.payload' },
      { id: 42, role: 'mcp.response.payload' },
    ],
  };

  assert.deepEqual(mcpPayloadEvidenceIds(action), [192, 196]);
});
