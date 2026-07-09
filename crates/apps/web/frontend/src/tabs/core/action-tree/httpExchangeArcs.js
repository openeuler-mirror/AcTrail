import { TREE_NODE_TYPES } from './config';

const HTTP_REQUEST_ACTION_ID_ATTR = 'http.request.action_id';
const HTTP_DIRECTION_ATTR = 'direction';
const HTTP_OPERATION_ATTR = 'http.operation';
const HTTP_OUTBOUND_DIRECTION = 'outbound';
const HTTP_INBOUND_DIRECTION = 'inbound';
const HTTP_REQUEST_OPERATION = 'request';
const HTTP_RESPONSE_OPERATION = 'response';

export function buildHttpExchangeArcOverlay(root, canvas) {
  if (!root || !canvas) {
    return emptyOverlay();
  }
  const actionNodes = visibleActionNodes(root);
  const actionNodeById = new Map(actionNodes.map((node) => [node.id, node]));
  const elementByActionId = visibleActionElementById(canvas);
  const arcs = [];
  for (const responseNode of actionNodes) {
    const responseAttrs = rawAttributes(responseNode);
    const requestActionId = responseAttrs[HTTP_REQUEST_ACTION_ID_ATTR];
    if (!requestActionId || !httpResponseNode(responseNode)) {
      continue;
    }
    const requestNode = actionNodeById.get(requestActionId);
    if (!requestNode || !httpRequestNode(requestNode)) {
      continue;
    }
    const requestElement = elementByActionId.get(requestActionId);
    const responseElement = elementByActionId.get(responseNode.id);
    if (!requestElement || !responseElement) {
      continue;
    }
    const arc = httpExchangeArc(canvas, requestNode, requestElement, responseNode, responseElement);
    if (arc) {
      arcs.push(arc);
    }
  }
  return {
    arcs,
    size: {
      width: Math.max(canvas.scrollWidth, canvas.clientWidth, 1),
      height: Math.max(canvas.scrollHeight, canvas.clientHeight, 1),
    },
  };
}

function emptyOverlay() {
  return {
    arcs: [],
    size: { width: 0, height: 0 },
  };
}

function visibleActionNodes(root) {
  const nodes = [];
  walkVisibleActionNodes(root, nodes);
  return nodes;
}

function walkVisibleActionNodes(node, nodes) {
  if (!node) {
    return;
  }
  if (node.nodeType === TREE_NODE_TYPES.action) {
    nodes.push(node);
  }
  for (const child of node.children ?? []) {
    walkVisibleActionNodes(child, nodes);
  }
}

function visibleActionElementById(canvas) {
  const elementById = new Map();
  for (const element of canvas.querySelectorAll('[data-action-node-id]')) {
    const id = element.dataset.actionNodeId;
    if (id) {
      elementById.set(id, element);
    }
  }
  return elementById;
}

function rawAttributes(node) {
  return node?.detail?.raw?.attributes ?? {};
}

function httpRequestNode(node) {
  const attrs = rawAttributes(node);
  return (
    node?.kind === 'http.message' &&
    attrs[HTTP_DIRECTION_ATTR] === HTTP_OUTBOUND_DIRECTION &&
    attrs[HTTP_OPERATION_ATTR] === HTTP_REQUEST_OPERATION
  );
}

function httpResponseNode(node) {
  const attrs = rawAttributes(node);
  return (
    node?.kind === 'http.message' &&
    attrs[HTTP_DIRECTION_ATTR] === HTTP_INBOUND_DIRECTION &&
    attrs[HTTP_OPERATION_ATTR] === HTTP_RESPONSE_OPERATION
  );
}

function httpExchangeArc(canvas, requestNode, requestElement, responseNode, responseElement) {
  const canvasRect = canvas.getBoundingClientRect();
  const requestRect = nodeCardRect(requestElement);
  const responseRect = nodeCardRect(responseElement);
  if (!requestRect || !responseRect) {
    return null;
  }
  return {
    id: `${requestNode.id}->${responseNode.id}`,
    path: exchangeArcPath(requestRect, responseRect, canvasRect),
  };
}

function nodeCardRect(nodeElement) {
  return (nodeElement.querySelector('.action-card') ?? nodeElement).getBoundingClientRect();
}

function exchangeArcPath(requestRect, responseRect, canvasRect) {
  const requestCenterX = requestRect.left + requestRect.width / 2;
  const responseCenterX = responseRect.left + responseRect.width / 2;
  const requestCenterY = requestRect.top + requestRect.height / 2;
  const responseCenterY = responseRect.top + responseRect.height / 2;
  const sameColumn = Math.abs(requestCenterX - responseCenterX) < 24;
  if (sameColumn) {
    const startX = requestRect.right - canvasRect.left;
    const startY = requestCenterY - canvasRect.top;
    const endX = responseRect.right - canvasRect.left;
    const endY = responseCenterY - canvasRect.top;
    const verticalDistance = Math.abs(endY - startY);
    const controlX = Math.max(startX, endX) + Math.min(112, 36 + verticalDistance * 0.18);
    return cubicPath(startX, startY, controlX, startY, controlX, endY, endX, endY);
  }
  const requestLeftOfResponse = requestCenterX < responseCenterX;
  const startX = (requestLeftOfResponse ? requestRect.right : requestRect.left) - canvasRect.left;
  const startY = requestCenterY - canvasRect.top;
  const endX = (requestLeftOfResponse ? responseRect.left : responseRect.right) - canvasRect.left;
  const endY = responseCenterY - canvasRect.top;
  const horizontalDistance = Math.abs(endX - startX);
  const controlOffset = Math.max(44, horizontalDistance * 0.45);
  const controlSign = requestLeftOfResponse ? 1 : -1;
  return cubicPath(
    startX,
    startY,
    startX + controlOffset * controlSign,
    startY,
    endX - controlOffset * controlSign,
    endY,
    endX,
    endY,
  );
}

function cubicPath(startX, startY, control1X, control1Y, control2X, control2Y, endX, endY) {
  return [
    `M ${roundSvgNumber(startX)} ${roundSvgNumber(startY)}`,
    `C ${roundSvgNumber(control1X)} ${roundSvgNumber(control1Y)}`,
    `${roundSvgNumber(control2X)} ${roundSvgNumber(control2Y)}`,
    `${roundSvgNumber(endX)} ${roundSvgNumber(endY)}`,
  ].join(' ');
}

function roundSvgNumber(value) {
  return Number(value).toFixed(1);
}
