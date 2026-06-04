const state = {
  traces: [],
  trace: null,
  actionTree: null,
  tab: "action_tree",
  selectedNodeId: null,
  expandedActions: new Set(),
};

const $ = (id) => document.getElementById(id);
const esc = (value) => String(value ?? "").replace(/[&<>"']/g, (ch) => ({
  "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;"
}[ch]));

async function api(path) {
  const response = await fetch(path);
  if (!response.ok) throw new Error(await response.text());
  return response.json();
}

async function loadTraces() {
  const data = await api("/api/traces");
  state.traces = data.traces;
  renderTraces();
  const selected = state.trace?.trace?.id ?? state.traces[0]?.id;
  if (selected) await loadTrace(selected);
}

async function loadTrace(id) {
  state.trace = await api(`/api/traces/${id}`);
  state.actionTree = await api(`/api/traces/${id}/action-tree`);
  state.selectedNodeId = null;
  state.expandedActions = new Set(state.actionTree.roots ?? []);
  renderTraces();
  renderTrace();
}

function renderTraces() {
  $("traces").innerHTML = state.traces.map((trace) => `
    <button class="trace-row ${state.trace?.trace?.id === trace.id ? "active" : ""}" data-trace="${trace.id}">
      <div class="trace-name">${esc(trace.name)}</div>
      <div class="subtle">${esc(trace.display_id)} · pid ${esc(trace.root_pid)} · ${esc(trace.state)}</div>
    </button>`).join("");
  document.querySelectorAll("[data-trace]").forEach((node) => {
    node.onclick = () => loadTrace(node.dataset.trace);
  });
}

function renderTrace() {
  const detail = state.trace;
  if (!detail) return;
  const trace = detail.trace;
  $("traceTitle").textContent = `${trace.display_id} · ${trace.name} · ${trace.state}/${trace.health}`;
  const metrics = detail.counts;
  $("metrics").innerHTML = [
    ["Events", metrics.events],
    ["Processes", detail.processes.length],
    ["Network", metrics.net],
    ["Files", metrics.file],
    ["IPC", metrics.ipc],
    ["Application", metrics.application],
    ["Resources", metrics.resource],
    ["Payload bytes", metrics.retained_payload_bytes],
    ["Labels", metrics.label],
    ["Diagnostics", detail.diagnostics.length],
  ].map(([label, value]) => `<div class="metric"><span class="subtle">${label}</span><strong>${esc(value)}</strong></div>`).join("");
  renderContent();
}

function renderContent() {
  if (state.tab === "action_tree") {
    renderActionTree();
    return;
  }
  renderTable();
}

function renderActionTree() {
  const tree = state.actionTree;
  if (!tree) {
    $("table").innerHTML = `<div class="empty">No semantic action data loaded.</div>`;
    setDetail("No selection", "");
    return;
  }
  const index = actionIndex();
  const children = childIndex(index);
  const query = $("search").value.toLowerCase();
  const visible = visibleActionIds(index, children, query);
  const roots = (tree.roots ?? []).filter((id) => index.has(id) && visible.has(id));
  if (!state.selectedNodeId && roots.length > 0) {
    state.selectedNodeId = actionNodeId(roots[0]);
  }
  const rows = roots.map((id) => renderActionNode(id, null, index, children, visible, 0)).join("");
  $("table").innerHTML = `<div class="tree">${rows || `<div class="empty">No matching semantic actions.</div>`}</div>`;
  bindActionTree();
  const selected = selectedAction(index);
  if (selected) {
    renderActionDetail(selected, index);
  } else if (!state.selectedNodeId) {
    setDetail("No selection", "");
  }
}

function actionIndex() {
  return new Map((state.actionTree?.actions ?? []).map((action) => [action.id, action]));
}

function childIndex(index) {
  const children = new Map();
  for (const link of state.actionTree?.links ?? []) {
    if (!index.has(link.parent) || !index.has(link.child)) continue;
    if (!children.has(link.parent)) children.set(link.parent, []);
    children.get(link.parent).push({ id: link.child, role: link.role });
  }
  for (const rows of children.values()) {
    rows.sort((left, right) => actionSortKey(index.get(left.id)).localeCompare(actionSortKey(index.get(right.id))));
  }
  return children;
}

function visibleActionIds(index, children, query) {
  const visible = new Set();
  const visit = (id) => {
    const action = index.get(id);
    if (!action) return false;
    const childHit = (children.get(id) ?? []).map((child) => visit(child.id)).some(Boolean);
    const selfHit = !query || JSON.stringify(action).toLowerCase().includes(query);
    if (selfHit || childHit) visible.add(id);
    return selfHit || childHit;
  };
  for (const id of state.actionTree?.roots ?? []) visit(id);
  return visible;
}

function renderActionNode(id, linkRole, index, children, visible, depth) {
  const action = index.get(id);
  if (!action || !visible.has(id)) return "";
  const childRows = (children.get(id) ?? []).filter((child) => visible.has(child.id));
  const evidenceRows = action.evidence ?? [];
  const expanded = state.expandedActions.has(id);
  const expandable = childRows.length > 0 || evidenceRows.length > 0;
  const marker = expandable ? (expanded ? "-" : "+") : "";
  const active = state.selectedNodeId === actionNodeId(id) ? " active" : "";
  let output = `<div class="tree-node${active}" style="--depth:${depth}" data-action-id="${esc(id)}">
    <div class="tree-node-title">${esc(marker)} ${esc(action.title || action.kind)}</div>
    <div class="tree-meta">${esc(actionTime(action))} · ${esc(action.kind)} · pid ${esc(action.process?.pid)} · ${esc(action.status)}${linkRole ? ` · ${esc(linkRole)}` : ""}</div>
  </div>`;
  if (!expanded) return output;
  output += childRows.map((child) => renderActionNode(child.id, child.role, index, children, visible, depth + 1)).join("");
  output += evidenceRows.map((evidence, index) => renderEvidenceNode(action, evidence, index, depth + 1)).join("");
  return output;
}

function renderEvidenceNode(action, evidence, index, depth) {
  const active = state.selectedNodeId === evidenceNodeId(action.id, index) ? " active" : "";
  const source = evidenceSource(evidence);
  const label = source?.summary || source?.title || `${evidence.kind} ${evidence.id}`;
  return `<div class="tree-node evidence-node${active}" style="--depth:${depth}" data-evidence-action="${esc(action.id)}" data-evidence-index="${index}">
    <div class="tree-node-title">${esc(evidence.kind)} ${esc(evidence.id)}</div>
    <div class="tree-meta">${esc(evidence.role)}${label ? ` · ${esc(label)}` : ""}</div>
  </div>`;
}

function bindActionTree() {
  document.querySelectorAll("[data-action-id]").forEach((node) => {
    node.onclick = () => {
      const id = node.dataset.actionId;
      state.selectedNodeId = actionNodeId(id);
      if (state.expandedActions.has(id)) {
        state.expandedActions.delete(id);
      } else {
        state.expandedActions.add(id);
      }
      renderActionTree();
    };
  });
  document.querySelectorAll("[data-evidence-action]").forEach((node) => {
    node.onclick = () => showEvidenceDetail(node.dataset.evidenceAction, Number(node.dataset.evidenceIndex)).catch(showError);
  });
}

function renderActionDetail(action, index) {
  const links = state.actionTree?.links ?? [];
  const relationRows = links
    .filter((link) => link.parent === action.id || link.child === action.id)
    .map((link) => {
      const parent = index.get(link.parent);
      const child = index.get(link.child);
      if (link.parent === action.id) return ["Child", `${link.role} -> ${child?.title ?? link.child}`];
      return ["Parent", `${link.role} <- ${parent?.title ?? link.parent}`];
    });
  setDetail(action.title || action.kind, [
    kvSection("Action", [
      ["Kind", action.kind],
      ["Status", action.status],
      ["Completeness", action.completeness],
      ["Time", actionTime(action)],
      ["PID", action.process?.pid],
      ["Action ID", action.id],
    ]),
    mapSection("Attributes", action.attributes),
    kvSection("Links", relationRows),
    kvSection("Evidence", (action.evidence ?? []).map((evidence) => [`${evidence.kind} ${evidence.id}`, evidence.role])),
    rawJson(action),
  ].join(""));
}

async function showEvidenceDetail(actionId, evidenceIndex) {
  const index = actionIndex();
  const action = index.get(actionId);
  const evidence = action?.evidence?.[evidenceIndex];
  if (!action || !evidence) return;
  state.selectedNodeId = evidenceNodeId(actionId, evidenceIndex);
  renderActionTree();
  if (evidence.kind === "payload_segment") {
    setDetail(`Payload ${evidence.id}`, `<div class="detail-section">Loading payload...</div>`);
    const payload = await api(`/api/traces/${state.trace.trace.id}/payloads/${evidence.id}`);
    setDetail(`Payload ${evidence.id}`, [
      kvSection("Evidence", evidenceRows(evidence)),
      kvSection("Payload", objectRows(payload)),
      rawJson(payload),
    ].join(""));
    return;
  }
  const source = evidenceSource(evidence);
  setDetail(`${evidence.kind} ${evidence.id}`, [
    kvSection("Evidence", evidenceRows(evidence)),
    source ? kvSection("Source", objectRows(source)) : "",
    rawJson({ evidence, source }),
  ].join(""));
}

function renderTable() {
  const detail = state.trace;
  if (!detail) return;
  const query = $("search").value.toLowerCase();
  const sourceRows = state.tab === "resources"
    ? detail.events.filter((row) => row.domain === "Resource")
    : detail[state.tab];
  const rows = sourceRows.filter((row) => JSON.stringify(row).toLowerCase().includes(query));
  const table = {
    timeline: renderTimeline,
    events: renderEvents,
    process_tree: renderProcessTree,
    processes: renderProcesses,
    payloads: renderPayloads,
    resources: renderResources,
    diagnostics: renderDiagnostics,
  }[state.tab](rows);
  $("table").innerHTML = table;
  if (rows[0]) {
    showRowDetail(rows[0]).catch(showError);
  } else {
    setDetail("No selection", "");
  }
  document.querySelectorAll("[data-row]").forEach((row) => {
    row.onclick = () => showRowDetail(rows[Number(row.dataset.row)]).catch(showError);
  });
}

async function showRowDetail(row) {
  if (state.tab !== "payloads" && !(state.tab === "timeline" && row.kind === "payload")) {
    setDetail(rowTitle(row), rawJson(row));
    return;
  }
  setDetail(`Payload ${row.id}`, `<div class="detail-section">Loading payload...</div>`);
  const segment = await api(`/api/traces/${state.trace.trace.id}/payloads/${row.id}`);
  setDetail(`Payload ${row.id}`, [
    kvSection("Payload", objectRows(segment)),
    rawJson(segment),
  ].join(""));
}

function renderTimeline(rows) {
  return table(["Time", "Lane", "PID", "Type", "Summary"], rows.map((row) => [
    row.observed_at, row.lane, row.pid, row.title, row.summary
  ]));
}

function renderEvents(rows) {
  return table(["Event", "Domain", "PID", "Operation", "Summary"], rows.map((row) => [
    row.display_id, row.domain, row.pid, row.operation, row.summary
  ]));
}

function renderProcessTree(rows) {
  return table(["PID", "Parent", "State", "Children", "Generation"], rows.map((row) => [
    "- ".repeat(row.depth) + row.pid,
    row.parent_pid ?? "", row.state, row.children, row.generation
  ]));
}

function renderProcesses(rows) {
  return table(["PID", "State", "Parent", "Exit", "Generation"], rows.map((row) => [
    row.pid, row.state, row.parent_pid ?? "", row.exit_code ?? "", row.identity.generation
  ]));
}

function renderPayloads(rows) {
  return table(["Segment", "PID", "Direction", "Source", "Protocol", "Bytes"], rows.map((row) => [
    row.display_id, row.pid, row.direction, row.source, row.protocol_hint ?? "", `${row.captured_size}/${row.original_size}`
  ]));
}

function renderResources(rows) {
  return table(["Event", "PID", "Scope", "Subject", "CPU", "RSS KB", "VSZ KB"], rows.map((row) => [
    row.display_id,
    row.pid,
    row.operation,
    row.metadata.subject ?? "",
    row.metadata.cpu_percent ?? "",
    row.metadata.rss_kb ?? "",
    row.metadata.virtual_memory_kb ?? ""
  ]));
}

function renderDiagnostics(rows) {
  return table(["Diagnostic", "Severity", "Kind", "Message"], rows.map((row) => [
    `diag-${row.id}`, row.severity, row.kind, row.message
  ]));
}

function table(headers, rows) {
  if (!rows.length) return `<div class="empty">No matching rows.</div>`;
  return `<table><thead><tr>${headers.map((header) => `<th>${esc(header)}</th>`).join("")}</tr></thead>
    <tbody>${rows.map((cells, index) => `<tr data-row="${index}">${cells.map((cell) => `<td>${esc(cell)}</td>`).join("")}</tr>`).join("")}</tbody></table>`;
}

function selectedAction(index) {
  if (!state.selectedNodeId?.startsWith("action:")) return null;
  return index.get(state.selectedNodeId.slice("action:".length)) ?? null;
}

function actionNodeId(id) {
  return `action:${id}`;
}

function evidenceNodeId(actionId, index) {
  return `evidence:${actionId}:${index}`;
}

function actionSortKey(action) {
  return `${action?.start_time ?? ""}:${action?.id ?? ""}`;
}

function actionTime(action) {
  return action.end_time == null ? `${action.start_time}` : `${action.start_time} -> ${action.end_time}`;
}

function evidenceSource(evidence) {
  if (evidence.kind === "event") return state.trace?.events?.find((row) => Number(row.id) === Number(evidence.id));
  if (evidence.kind === "payload_segment") return state.trace?.payloads?.find((row) => Number(row.id) === Number(evidence.id));
  return null;
}

function evidenceRows(evidence) {
  return [
    ["Kind", evidence.kind],
    ["ID", evidence.id],
    ["Role", evidence.role],
  ];
}

function rowTitle(row) {
  return row.display_id ?? row.title ?? row.kind ?? row.domain ?? `row ${row.id ?? ""}`;
}

function objectRows(value) {
  return Object.entries(value ?? {}).map(([key, item]) => [
    key,
    item && typeof item === "object" ? JSON.stringify(item) : item,
  ]);
}

function mapSection(title, value) {
  return kvSection(title, objectRows(value));
}

function kvSection(title, rows) {
  if (!rows.length) return "";
  return `<div class="detail-section"><h3>${esc(title)}</h3><div class="kv">${
    rows.map(([key, value]) => `<div>${esc(key)}</div><div>${esc(value)}</div>`).join("")
  }</div></div>`;
}

function rawJson(value) {
  return `<details class="raw-json"><summary>Raw JSON</summary><pre>${esc(JSON.stringify(value, null, 2))}</pre></details>`;
}

function setDetail(title, body) {
  $("detailTitle").textContent = title;
  $("detail").innerHTML = body;
}

function showError(error) {
  setDetail("Error", `<div class="detail-section">${esc(error.message ?? error)}</div>`);
}

document.querySelectorAll(".tab").forEach((tab) => {
  tab.onclick = () => {
    document.querySelectorAll(".tab").forEach((node) => node.classList.remove("active"));
    tab.classList.add("active");
    state.tab = tab.dataset.tab;
    state.selectedNodeId = null;
    renderContent();
  };
});

$("refresh").onclick = loadTraces;
$("search").oninput = renderContent;
loadTraces().catch(showError);
