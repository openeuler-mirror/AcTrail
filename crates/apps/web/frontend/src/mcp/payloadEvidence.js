export function mcpPayloadEvidenceRole(kind) {
  if (kind === 'mcp.request') {
    return 'mcp.request.payload';
  }
  if (kind === 'mcp.response') {
    return 'mcp.response.payload';
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
