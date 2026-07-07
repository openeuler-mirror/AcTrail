export function mcpPayloadEvidenceRole(kind) {
  if (kind === 'mcp.client_send') {
    return 'mcp.client_send.payload';
  }
  if (kind === 'mcp.client_receive') {
    return 'mcp.client_receive.payload';
  }
  if (kind === 'mcp.stdin') {
    return 'mcp.stdin.payload';
  }
  if (kind === 'mcp.stdout') {
    return 'mcp.stdout.payload';
  }
  return null;
}

export function mcpPayloadEvidenceIds(action) {
  const role = mcpPayloadEvidenceRole(action?.kind);
  if (!role) {
    return [];
  }
  return (action.evidence ?? [])
    .filter((item) => item?.role === role)
    .map((item) => item?.id)
    .filter((id) => id !== null && id !== undefined);
}
