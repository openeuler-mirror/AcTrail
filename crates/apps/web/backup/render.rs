//! Static web assets.

pub fn html() -> String {
    r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>AcTrail</title>
  <link rel="stylesheet" href="/assets/app.css">
</head>
<body>
  <header class="topbar">
    <div>
      <h1>AcTrail</h1>
      <div id="traceTitle" class="subtle"></div>
    </div>
    <div class="toolbar">
      <input id="search" type="search" placeholder="Filter">
      <button id="refresh" type="button">Refresh</button>
    </div>
  </header>
  <main class="layout">
    <aside class="sidebar">
      <div class="section-title">Traces</div>
      <div id="traces" class="trace-list"></div>
    </aside>
    <section class="workspace">
      <div id="metrics" class="metrics"></div>
      <nav class="tabs">
        <button class="tab active" data-tab="timeline" type="button">Timeline</button>
        <button class="tab" data-tab="events" type="button">Events</button>
        <button class="tab" data-tab="process_tree" type="button">Process Tree</button>
        <button class="tab" data-tab="processes" type="button">Processes</button>
        <button class="tab" data-tab="payloads" type="button">Payloads</button>
        <button class="tab" data-tab="resources" type="button">Resources</button>
        <button class="tab" data-tab="diagnostics" type="button">Diagnostics</button>
      </nav>
      <div id="table" class="table-wrap"></div>
      <pre id="detail" class="detail"></pre>
    </section>
  </main>
  <script src="/assets/app.js"></script>
</body>
</html>
"#
    .to_string()
}

pub fn css() -> String {
    r#":root {
  --surface: #ffffff;
  --background: #f5f7fa;
  --line: #d8dee8;
  --text: #17202a;
  --muted: #667085;
  --accent: #1264a3;
  --ok: #1d7f4f;
  --warn: #a15c00;
  --radius: 6px;
}
* { box-sizing: border-box; }
body {
  margin: 0;
  background: var(--background);
  color: var(--text);
  font: 14px/1.45 system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
}
.topbar {
  height: 72px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 0 20px;
  border-bottom: 1px solid var(--line);
  background: var(--surface);
}
h1 { margin: 0; font-size: 22px; font-weight: 650; letter-spacing: 0; }
.subtle { color: var(--muted); font-size: 13px; }
.toolbar { display: flex; gap: 8px; align-items: center; }
input, button {
  border: 1px solid var(--line);
  background: var(--surface);
  color: var(--text);
  border-radius: var(--radius);
  height: 34px;
}
input { min-width: 240px; padding: 0 10px; }
button { padding: 0 12px; cursor: pointer; }
button:hover, .tab.active { border-color: var(--accent); color: var(--accent); }
.layout {
  display: grid;
  grid-template-columns: minmax(260px, 320px) 1fr;
  min-height: calc(100vh - 72px);
}
.sidebar {
  border-right: 1px solid var(--line);
  background: var(--surface);
  padding: 14px;
  overflow: auto;
}
.section-title {
  color: var(--muted);
  font-size: 12px;
  text-transform: uppercase;
  letter-spacing: .04em;
  margin-bottom: 8px;
}
.trace-row {
  width: 100%;
  text-align: left;
  height: auto;
  padding: 9px 10px;
  margin-bottom: 6px;
  border-radius: var(--radius);
}
.trace-row.active { border-color: var(--accent); background: #eef6fc; }
.trace-name { font-weight: 650; overflow-wrap: anywhere; }
.workspace { padding: 16px; min-width: 0; }
.metrics {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(126px, 1fr));
  gap: 8px;
  margin-bottom: 12px;
}
.metric {
  background: var(--surface);
  border: 1px solid var(--line);
  border-radius: var(--radius);
  padding: 10px;
}
.metric strong { display: block; font-size: 20px; }
.tabs { display: flex; gap: 8px; margin-bottom: 12px; }
.table-wrap {
  border: 1px solid var(--line);
  background: var(--surface);
  border-radius: var(--radius);
  overflow: auto;
}
table { width: 100%; border-collapse: collapse; min-width: 760px; }
th, td { padding: 9px 10px; border-bottom: 1px solid var(--line); text-align: left; vertical-align: top; }
th { color: var(--muted); font-size: 12px; font-weight: 650; background: #fafbfc; }
tr:hover td { background: #f8fbfd; }
.pill { color: var(--ok); font-weight: 650; }
.warn { color: var(--warn); font-weight: 650; }
.detail {
  margin: 12px 0 0;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  background: #101820;
  color: #e8edf2;
  padding: 12px;
  min-height: 96px;
  overflow: auto;
}
@media (max-width: 760px) {
  .topbar { height: auto; align-items: stretch; flex-direction: column; padding: 12px; }
  .toolbar { align-items: stretch; }
  input { min-width: 0; width: 100%; }
  .layout { grid-template-columns: 1fr; }
  .sidebar { border-right: 0; border-bottom: 1px solid var(--line); }
}
"#
    .to_string()
}

pub fn javascript() -> String {
    r#"const state = { traces: [], trace: null, tab: "timeline" };

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
  renderTable();
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
  $("detail").textContent = rows[0] ? JSON.stringify(rows[0], null, 2) : "";
  document.querySelectorAll("[data-row]").forEach((row) => {
    row.onclick = () => showRowDetail(rows[Number(row.dataset.row)]);
  });
}

async function showRowDetail(row) {
  if (state.tab !== "payloads" && !(state.tab === "timeline" && row.kind === "payload")) {
    $("detail").textContent = JSON.stringify(row, null, 2);
    return;
  }
  $("detail").textContent = "Loading payload...";
  const segment = await api(`/api/traces/${state.trace.trace.id}/payloads/${row.id}`);
  $("detail").textContent = JSON.stringify(segment, null, 2);
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
  return `<table><thead><tr>${headers.map((header) => `<th>${esc(header)}</th>`).join("")}</tr></thead>
    <tbody>${rows.map((cells, index) => `<tr data-row="${index}">${cells.map((cell) => `<td>${esc(cell)}</td>`).join("")}</tr>`).join("")}</tbody></table>`;
}

document.querySelectorAll(".tab").forEach((tab) => {
  tab.onclick = () => {
    document.querySelectorAll(".tab").forEach((node) => node.classList.remove("active"));
    tab.classList.add("active");
    state.tab = tab.dataset.tab;
    renderTable();
  };
});

$("refresh").onclick = loadTraces;
$("search").oninput = renderTable;
loadTraces().catch((error) => $("detail").textContent = error.message);
"#
    .to_string()
}
