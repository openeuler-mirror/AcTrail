// Normalizes field names across OpenCode event and permission payloads.

function property(properties, ...names) {
  for (const name of names) {
    const value = properties?.[name];
    if (value !== undefined && value !== null && String(value) !== "") return String(value);
  }
  return undefined;
}

export function sessionId(properties) {
  return property(properties, "sessionID", "sessionId", "session_id")
    || property(properties?.info, "sessionID", "sessionId", "session_id")
    || property(properties?.message, "sessionID", "sessionId", "session_id");
}

export function requestId(properties) {
  return property(properties, "id", "permissionID", "permissionId", "permission_id", "requestID", "requestId", "request_id");
}

export function userMessageId(properties) {
  const message = properties?.info || properties?.message || properties;
  return property(message, "id", "messageID", "messageId", "message_id");
}

export function key(session, request) {
  return `${session || "-"}\u0000${request}`;
}

export function permissionKey(permission) {
  return key(sessionId(permission), requestId(permission));
}
