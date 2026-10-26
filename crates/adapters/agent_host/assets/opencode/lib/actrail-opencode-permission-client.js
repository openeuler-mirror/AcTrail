// Lists OpenCode permissions for reconciliation.

import { permissionKey } from "./actrail-opencode-utils.js";

async function permissionList(client, path, directory, v2 = false) {
  const transport = client?._client;
  if (!transport?.get) {
    throw new Error("OpenCode plugin client does not expose an HTTP transport");
  }
  const result = await transport.get({
    url: path,
    ...(directory ? {
      query: v2 ? { location: { directory } } : { directory },
    } : {}),
  });
  if (result.error) {
    throw new Error(`${path} failed: ${JSON.stringify(result.error)}`);
  }
  const body = result.data;
  const permissions = v2 ? body?.data : body;
  if (!Array.isArray(permissions)) throw new Error(`${path} returned an invalid response`);
  return permissions;
}

export async function listPendingPermissions(client, directory) {
  const results = await Promise.allSettled([
    permissionList(client, "/api/permission/request", directory, true),
    permissionList(client, "/permission", directory),
  ]);
  const permissions = [];
  const seen = new Set();
  const failures = [];
  for (const result of results) {
    if (result.status === "rejected") {
      failures.push(result.reason);
      continue;
    }
    for (const permission of result.value) {
      const itemKey = permissionKey(permission);
      if (!seen.has(itemKey)) {
        seen.add(itemKey);
        permissions.push(permission);
      }
    }
  }
  if (permissions.length || failures.length < results.length) return permissions;
  throw new AggregateError(failures, "OpenCode permission reconciliation endpoints failed");
}
