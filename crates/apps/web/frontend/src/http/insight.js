import { chip, compactChips, compactRows, firstPresent } from '../detail/insight.js';

export function buildHttpDetailInsight(detail) {
  const action = detail?.raw ?? null;
  const attrs = action?.attributes ?? {};
  if (!action || !hasHttpSignal(action.kind, attrs)) {
    return null;
  }

  const method = firstPresent(attrs['http.request.method'], attrs.method);
  const statusCode = firstPresent(attrs['http.response.status_code'], attrs.status_code);
  const reason = firstPresent(attrs['http.response.reason'], attrs.reason);
  const operation = firstPresent(attrs['http.operation'], inferOperation(method, statusCode));
  const direction = attrs.direction;
  const protocol = firstPresent(
    attrs['http.request.protocol'],
    attrs['http.response.protocol'],
    attrs['network.protocol.version'],
  );
  const host = firstPresent(attrs['server.address'], attrs.host);
  const endpoint = firstPresent(attrs['url.path'], requestTarget(attrs.target, method));
  const scheme = firstPresent(attrs['url.scheme'], schemeFromBoundary(attrs.source_boundary));
  const endpointUrl = endpointUrlFromParts({ scheme, host, endpoint, method, target: attrs.target });
  const website = websiteFromParts({ scheme, host, method, target: attrs.target });
  const isResponse = operation === 'response' || Boolean(statusCode);
  const blocks = compactBlocks([
    endpointBlock({
      method,
      website,
      endpoint,
      endpointUrl,
      contentType: isResponse ? null : attrs.content_type,
      contentLength: isResponse ? null : attrs.content_length,
      streamKey: attrs['payload.stream_key'] ?? attrs.stream_key,
      sourceBoundary: attrs['payload.source_boundary'] ?? attrs.source_boundary,
    }),
    statusBlock({
      statusCode,
      reason,
      contentType: isResponse ? attrs.content_type : null,
      contentLength: isResponse ? attrs.content_length : null,
      bodyFormat: attrs['http.response.body_format'],
    }),
  ]);

  return {
    kind: action.kind,
    heading: 'HTTP Access',
    chips: compactChips([
      chip('method', method),
      chip('status', statusCode),
      chip('direction', direction),
      chip('protocol', protocol),
    ]),
    blocks,
  };
}

function hasHttpSignal(kind, attrs) {
  return (
    kind === 'http.message' ||
    attrs['http.operation'] ||
    attrs['http.request.method'] ||
    attrs['http.response.status_code'] ||
    attrs['network.protocol.name'] === 'http'
  );
}

function endpointBlock({
  method,
  website,
  endpoint,
  endpointUrl,
  contentType,
  contentLength,
  streamKey,
  sourceBoundary,
}) {
  if (!website && !endpoint && !endpointUrl) {
    return null;
  }
  const rows = compactRows({
    website,
    endpoint_url: endpointUrl,
    endpoint,
    content_type: contentType,
    content_length: contentLength,
    stream_key: streamKey,
    source_boundary: sourceBoundary,
  });
  return {
    id: 'http-endpoint',
    tone: 'http',
    label: 'Endpoint',
    title: method ? `${method} ${endpoint ?? endpointUrl ?? website}` : (endpointUrl ?? website ?? 'http access'),
    rows,
  };
}

function statusBlock({ statusCode, reason, contentType, contentLength, bodyFormat }) {
  if (!statusCode && !reason && !bodyFormat) {
    return null;
  }
  const rows = compactRows({
    status_code: statusCode,
    reason,
    content_type: contentType,
    content_length: contentLength,
    body_format: bodyFormat,
  });
  if (!rows.length) {
    return null;
  }
  return {
    id: 'http-status',
    tone: 'status',
    label: 'Response',
    title: statusCode ? `${statusCode}${reason ? ` ${reason}` : ''}` : 'response metadata',
    rows,
  };
}

function inferOperation(method, statusCode) {
  if (method) {
    return 'request';
  }
  if (statusCode) {
    return 'response';
  }
  return null;
}

function requestTarget(target, method) {
  if (!target) {
    return null;
  }
  if (method === 'CONNECT') {
    return null;
  }
  return String(target).startsWith('/') ? target : null;
}

function endpointUrlFromParts({ scheme, host, endpoint, method, target }) {
  if (method === 'CONNECT') {
    return connectUrl(scheme, target ?? host);
  }
  if (host && endpoint) {
    return `${scheme ?? 'http'}://${host}${endpoint.startsWith('/') ? endpoint : `/${endpoint}`}`;
  }
  if (target && absoluteUrl(target)) {
    return target;
  }
  return null;
}

function websiteFromParts({ scheme, host, method, target }) {
  const targetHost = method === 'CONNECT' ? target : host;
  if (!targetHost) {
    return null;
  }
  return `${scheme ?? 'http'}://${targetHost}`;
}

function connectUrl(scheme, target) {
  if (!target) {
    return null;
  }
  return `${scheme ?? schemeFromConnectTarget(target)}://${target}`;
}

function schemeFromBoundary(boundary) {
  if (boundary === 'TlsUserSpace') {
    return 'https';
  }
  return null;
}

function schemeFromConnectTarget(target) {
  return String(target).endsWith(':443') ? 'https' : 'http';
}

function absoluteUrl(value) {
  return /^https?:\/\//i.test(String(value));
}

function compactBlocks(blocks) {
  return blocks.filter(Boolean);
}
