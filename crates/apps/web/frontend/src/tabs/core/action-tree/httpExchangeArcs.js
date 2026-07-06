import { TREE_NODE_TYPES } from './config.js';
import {
  classifyMcpMessage,
  mcpJsonRpcPairKey,
} from '../../../mcp/messageClassification.js';

const HTTP_REQUEST_ACTION_ID_ATTR = 'http.request.action_id';
const HTTP_DIRECTION_ATTR = 'direction';
const HTTP_OPERATION_ATTR = 'http.operation';
const HTTP_OUTBOUND_DIRECTION = 'outbound';
const HTTP_INBOUND_DIRECTION = 'inbound';
const HTTP_REQUEST_OPERATION = 'request';
const HTTP_RESPONSE_OPERATION = 'response';
const HTTP_ARC_CLASS = 'http-exchange-arc';

const MCP_ARC_CLASS = 'mcp-exchange-arc';

export function buildHttpExchangeArcOverlay(root, canvas) {
  return buildMessagePairArcOverlay(root, canvas);
}

export function buildMessagePairArcOverlay(root, canvas) {
  if (!root || !canvas) {
    return emptyOverlay();
  }
  const actionNodes = visibleActionNodes(root);
  const actionNodeById = new Map(actionNodes.map((node) => [node.id, node]));
  const elementByActionId = visibleActionElementById(canvas);
  const arcs = [
    ...httpExchangeArcs({ actionNodes, actionNodeById, elementByActionId, canvas }),
    ...mcpJsonRpcExchangeArcs({ actionNodes, elementByActionId, canvas }),
  ];
  return {
    arcs,
    size: {
      width: Math.max(canvas.scrollWidth, canvas.clientWidth, 1),
      height: Math.max(canvas.scrollHeight, canvas.clientHeight, 1),
    },
  };
}

function httpExchangeArcs({ actionNodes, actionNodeById, elementByActionId, canvas }) {
  const arcs = [];
  for (const targetNode of actionNodes) {
    const targetAttrs = rawAttributes(targetNode);
    const sourceActionId = targetAttrs[HTTP_REQUEST_ACTION_ID_ATTR];
    if (!sourceActionId || !httpResponseNode(targetNode)) {
      continue;
    }
    const sourceNode = actionNodeById.get(sourceActionId);
    if (!sourceNode || !httpRequestNode(sourceNode)) {
      continue;
    }
    const sourceElement = elementByActionId.get(sourceActionId);
    const targetElement = elementByActionId.get(targetNode.id);
    if (!sourceElement || !targetElement) {
      continue;
    }
    const arc = messagePairArc({
      canvas,
      sourceNode,
      sourceElement,
      targetNode,
      targetElement,
      className: HTTP_ARC_CLASS,
    });
    if (arc) {
      arcs.push(arc);
    }
  }
  return arcs;
}

function mcpJsonRpcExchangeArcs({ actionNodes, elementByActionId, canvas }) {
  const requestNodesByKey = new Map();
  for (const node of actionNodes) {
    const attrs = rawAttributes(node);
    const classification = classifyMcpMessage(node.kind, attrs);
    if (!classification.isTransportMessage || classification.jsonRpcRole !== 'request') {
      continue;
    }
    const key = mcpJsonRpcPairKey(attrs);
    if (!key) {
      continue;
    }
    const nodes = requestNodesByKey.get(key) ?? [];
    nodes.push(node);
    requestNodesByKey.set(key, nodes);
  }

  const arcs = [];
  const pairedRequestIds = new Set();
  for (const targetNode of actionNodes) {
    const attrs = rawAttributes(targetNode);
    const classification = classifyMcpMessage(targetNode.kind, attrs);
    if (!classification.isTransportMessage || classification.jsonRpcRole !== 'response') {
      continue;
    }
    const key = mcpJsonRpcPairKey(attrs);
    if (!key) {
      continue;
    }
    const sourceNode = (requestNodesByKey.get(key) ?? []).find((node) => !pairedRequestIds.has(node.id));
    if (!sourceNode) {
      continue;
    }
    const sourceElement = elementByActionId.get(sourceNode.id);
    const targetElement = elementByActionId.get(targetNode.id);
    if (!sourceElement || !targetElement) {
      continue;
    }
    const arc = messagePairArc({
      canvas,
      sourceNode,
      sourceElement,
      targetNode,
      targetElement,
      className: MCP_ARC_CLASS,
    });
    if (arc) {
      arcs.push(arc);
      pairedRequestIds.add(sourceNode.id);
    }
  }
  return arcs;
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

function messagePairArc({ canvas, sourceNode, sourceElement, targetNode, targetElement, className }) {
  const canvasRect = canvas.getBoundingClientRect();
  const sourceRect = nodeCardRect(sourceElement);
  const targetRect = nodeCardRect(targetElement);
  if (!sourceRect || !targetRect) {
    return null;
  }
  return {
    id: `${sourceNode.id}->${targetNode.id}`,
    className,
    path: exchangeArcPath(sourceRect, targetRect, canvasRect),
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
