//! Static web assets for actrailweb with latency analysis and full upstream features.

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
      <div id="traceSelectorContainer"></div>
      <div class="section-title">Trace Info</div>
      <div id="traceInfo" class="trace-info"></div>
      <div class="section-title" style="margin-top:16px">Event Stats</div>
      <div id="eventStats" class="quick-stats"></div>
      <div class="section-title" style="margin-top:16px">System Stats</div>
      <div id="systemStats" class="quick-stats"></div>
    </aside>
    <section class="workspace">
      <div id="metrics" class="metrics"></div>
      <nav class="tabs">
        <button class="tab active" data-tab="overview" type="button">Overview</button>
        <button class="tab" data-tab="commands" type="button">Commands</button>
        <button class="tab" data-tab="timeline" type="button">Timeline</button>
        <button class="tab" data-tab="events" type="button">Events</button>
        <button class="tab" data-tab="process_tree" type="button">Process Tree</button>
        <button class="tab" data-tab="processes" type="button">Processes</button>
        <button class="tab" data-tab="network" type="button">Network</button>
        <button class="tab" data-tab="files" type="button">Files</button>
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
"#.to_string()
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
  --error: #c0392b;
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
.toolbar { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
input, button {
  border: 1px solid var(--line);
  background: var(--surface);
  color: var(--text);
  border-radius: var(--radius);
  height: 34px;
}
input { min-width: 180px; padding: 0 10px; }
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
.trace-info { font-size: 13px; line-height: 1.6; }
.trace-info div { margin-bottom: 4px; }
.quick-stats { font-size: 13px; line-height: 1.6; }
.quick-stats div { margin-bottom: 4px; }
.workspace { padding: 16px; min-width: 0; }
.metrics {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(100px, 1fr));
  gap: 8px;
  margin-bottom: 12px;
}
.metric {
  background: var(--surface);
  border: 1px solid var(--line);
  border-radius: var(--radius);
  padding: 8px;
}
.metric strong { display: block; font-size: 18px; }
.metric-label { color: var(--muted); font-size: 11px; }
.tabs { display: flex; gap: 4px; margin-bottom: 12px; flex-wrap: wrap; }
.tab { font-size: 12px; padding: 4px 8px; }
.table-wrap {
  border: 1px solid var(--line);
  background: var(--surface);
  border-radius: var(--radius);
  overflow: auto;
  max-height: calc(100vh - 280px);
}
.table-controls { margin-bottom: 8px; display: flex; gap: 8px; }
table { width: 100%; border-collapse: collapse; min-width: 800px; }
th, td { padding: 8px 10px; border-bottom: 1px solid var(--line); text-align: left; vertical-align: top; font-size: 13px; }
th { color: var(--muted); font-size: 12px; font-weight: 650; background: #fafbfc; }
tr:hover td { background: #f8fbfd; }
.tree-toggle { cursor: pointer; user-select: none; margin-right: 4px; color: var(--muted); }
.tree-toggle:hover { color: var(--accent); }
.tree-indent { color: var(--line); }
.cmd-tooltip { position: relative; cursor: help; }
.cmd-tooltip:hover::after {
  content: attr(data-full);
  position: absolute;
  left: 0; top: 100%;
  background: var(--surface);
  border: 1px solid var(--line);
  padding: 4px 8px;
  border-radius: var(--radius);
  white-space: pre-wrap;
  max-width: 600px;
  z-index: 100;
  font-size: 12px;
  box-shadow: 2px 2px 4px rgba(0,0,0,0.1);
}
.pill { color: var(--ok); font-weight: 650; }
.warn { color: var(--warn); font-weight: 650; }
.error { color: var(--error); font-weight: 650; }
.detail {
  margin: 12px 0 0;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  background: #101820;
  color: #e8edf2;
  padding: 12px;
  font-size: 12px;
  min-height: 60px;
  max-height: 200px;
  overflow: auto;
}
.tree-node {
  padding: 8px 10px;
  cursor: pointer;
  border-bottom: 1px solid var(--line);
}
.tree-node:hover { background: #f8fbfd; }
.tree-node.active { background: #eef6fc; border-color: var(--accent); }
.tree-node-title { font-weight: 650; }
.tree-meta { color: var(--muted); font-size: 12px; }
@media (max-width: 900px) {
  .topbar { height: auto; align-items: stretch; flex-direction: column; padding: 12px; }
  .toolbar { align-items: stretch; }
  input { min-width: 0; width: 100%; }
  .layout { grid-template-columns: 1fr; }
  .sidebar { border-right: 0; border-bottom: 1px solid var(--line); }
}
"#.to_string()
}

pub fn javascript() -> String {
    r#"const state = { data: null, tab: "overview", traces: null, currentTraceId: null };
const collapsedNodes = new Set();
const collapsedCmds = new Set();

const $ = (id) => document.getElementById(id);
const esc = (value) => String(value ?? "").replace(/[&<>"']/g, (ch) => ({
  "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;"
}[ch]));

function formatDuration(ms) {
  if (ms === null || ms === undefined) return "N/A";
  const value = parseFloat(ms);
  if (!Number.isFinite(value)) return "N/A";
  if (value < 1) return value.toFixed(3) + "ms";
  if (value < 1000) return value.toFixed(1) + "ms";
  const seconds = Math.floor(value / 1000);
  const millis = Math.round(value % 1000);
  if (seconds < 60) return seconds + "s " + millis + "ms";
  const minutes = Math.floor(seconds / 60);
  const secs = seconds % 60;
  return minutes + "m " + secs + "s";
}

function formatUnixTimestamp(unixSec) {
  if (unixSec === null || unixSec === undefined) return "N/A";
  const date = new Date(unixSec * 1000);
  return date.toISOString().substr(11, 8) + " UTC";
}

function formatUnixMillis(millis) {
  if (millis === null || millis === undefined) return "N/A";
  const date = new Date(millis);
  const time = date.toISOString().substr(11, 12);
  return time + " UTC";
}

function getLatencyClass(ms, maxMs) {
  if (!ms || !maxMs || maxMs === 0) return "";
  const ratio = ms / maxMs;
  if (ratio > 0.8) return "error";
  if (ratio > 0.5) return "warn";
  return "";
}

async function api(path) {
  const response = await fetch(path);
  if (!response.ok) throw new Error(await response.text());
  return response.json();
}

async function loadData() {
  try {
    const tracesData = await api("/api/traces");
    if (tracesData.traces && tracesData.traces.length > 0) {
      state.traces = tracesData.traces;
      state.currentTraceId = tracesData.traces[0].id || tracesData.traces[0].trace_id;
      renderTraceSelector();
    }
  } catch (e) {}
  await loadTraceData();
}

async function loadTraceData() {
  const traceId = state.currentTraceId || 1;
  const data = await api("/api/traces/" + traceId);
  state.data = data;
  renderTraceInfo();
  renderEventStats();
  renderSystemStats();
  renderMetrics();
  renderTable();
}

function renderTraceSelector() {
  const traces = state.traces;
  if (!traces || traces.length === 0) return;
  let options = '';
  traces.forEach(t => {
    const tid = t.id || t.trace_id;
    const name = t.display_name || t.name || 'trace-' + tid;
    const selected = tid == state.currentTraceId ? ' selected' : '';
    options += `<option value="${tid}"${selected}>${esc(name)} (trace-${tid}) - ${t.state || t.lifecycle_state || 'Unknown'}</option>`;
  });
  $("traceSelectorContainer").innerHTML = `
    <div style="margin-bottom: 12px;">
      <label for="traceSelector" style="font-weight: bold; margin-right: 8px;">Select Trace:</label>
      <select id="traceSelector" onchange="selectTrace(this.value)" style="padding: 4px 8px; border-radius: 4px; border: 1px solid #ccc;">
        ${options}
      </select>
    </div>
  `;
}

function selectTrace(traceId) {
  state.currentTraceId = parseInt(traceId);
  loadTraceData();
}

function renderTraceInfo() {
  const data = state.data;
  if (!data) return;
  const trace = data.trace || {};
  $("traceInfo").innerHTML = `
    <div><strong>Trace ID:</strong> ${esc(trace.display_id || trace.id || 'N/A')}</div>
    <div><strong>Profile:</strong> ${esc(trace.profile || 'N/A')}</div>
    <div><strong>Completeness:</strong> <span class="${trace.health === 'Clean' ? 'pill' : 'warn'}">${esc(trace.health || 'N/A')}</span></div>
  `;
}

function renderEventStats() {
  const data = state.data;
  if (!data) return;
  const events = data.events || [];
  const counts = {};
  events.forEach(e => counts[e.domain || e.lane || 'Unknown'] = (counts[e.domain || e.lane || 'Unknown'] || 0) + 1);
  let html = '';
  for (const [d, c] of Object.entries(counts).sort((a,b) => b[1] - a[1])) {
    html += `<div><strong>${esc(d)}:</strong> ${c}</div>`;
  }
  $("eventStats").innerHTML = html || '<div class="subtle">No events</div>';
}

function renderSystemStats() {
  const data = state.data;
  if (!data) return;
  const processes = data.processes || [];
  const active = processes.filter(p => p.state === 'Running' || p.state === 'Active').length;
  const exited = processes.filter(p => p.state === 'Exited').length;
  $("systemStats").innerHTML = `
    <div><strong>Processes:</strong> ${processes.length}</div>
    <div><strong>Active:</strong> <span class="pill">${active}</span></div>
    <div><strong>Exited:</strong> ${exited}</div>
    <div><strong>Events:</strong> ${(data.events || []).length}</div>
  `;
}

function renderMetrics() {
  const data = state.data;
  if (!data) return;
  const counts = data.counts || {};
  const processes = data.processes || [];
  const events = data.events || [];
  const diagnostics = data.diagnostics || [];
  $("metrics").innerHTML = `
    <div class="metric"><span class="metric-label">Processes</span><strong>${processes.length}</strong></div>
    <div class="metric"><span class="metric-label">Events</span><strong>${events.length}</strong></div>
    <div class="metric"><span class="metric-label">Network</span><strong>${counts.net || 0}</strong></div>
    <div class="metric"><span class="metric-label">Files</span><strong>${counts.file || 0}</strong></div>
    <div class="metric"><span class="metric-label">IPC</span><strong>${counts.ipc || 0}</strong></div>
    <div class="metric"><span class="metric-label">Resources</span><strong>${counts.resource || 0}</strong></div>
    <div class="metric"><span class="metric-label">Diags</span><strong>${diagnostics.length}</strong></div>
  `;
}

function renderTable() {
  const data = state.data;
  if (!data) return;
  const query = $("search").value.toLowerCase();
  const funcs = {
    overview: renderOverview,
    commands: renderCommands,
    timeline: renderTimeline,
    events: renderEvents,
    process_tree: renderProcessTree,
    processes: renderProcesses,
    network: renderNetwork,
    files: renderFiles,
    payloads: renderPayloads,
    resources: renderResources,
    diagnostics: renderDiagnostics,
  };
  funcs[state.tab](data, query);
}

function renderOverview(data, q) {
  const commands = data.analysis && data.analysis.commands ? data.analysis.commands : [];
  const sorted = [...commands].sort((a, b) => (b.duration_ms || 0) - (a.duration_ms || 0)).slice(0, 10);
  const max = sorted.length > 0 ? sorted[0].duration_ms : 100;
  let html = `<h3 style="margin:0 0 8px;font-size:14px;">Slowest Commands (Top 10)</h3>`;
  html += `<table><thead><tr><th>PID</th><th>Command</th><th>Duration</th><th>Exit</th></tr></thead><tbody>`;
  sorted.forEach(c => {
    const dur = c.duration_ms !== null && c.duration_ms !== undefined ? formatDuration(c.duration_ms) : 'N/A';
    // Truncate command with hover tooltip
    const cmdText = c.command || '';
    const cmdDisplay = cmdText.length > 50 ? cmdText.substring(0, 50) + '...' : cmdText;
    const cmdFull = cmdText.length > 50 ? `data-full="${esc(cmdText)}"` : '';
    html += `<tr><td>${esc(c.pid)}</td><td class="cmd-tooltip" ${cmdFull}>${esc(cmdDisplay)}</td><td class="${getLatencyClass(c.duration_ms, max)}">${dur}</td><td>${c.exit_code !== null && c.exit_code !== undefined ? esc(c.exit_code) : '-'}</td></tr>`;
  });
  html += '</tbody></table>';
  $("table").innerHTML = html;
  $("detail").textContent = '';
}

function renderCommands(data, q) {
  const commands = data.analysis && data.analysis.commands ? data.analysis.commands : [];
  if (!commands || commands.length === 0) {
    $("table").innerHTML = '<div class="subtle">No commands found</div>';
    return;
  }
  
  const durations = commands.map(c => c.duration_ms || 0).filter(d => d > 0);
  const max = durations.length > 0 ? Math.max(...durations) : 0;
  
  // Build parent->children map and find roots
  const cmdChildrenMap = {};
  const allPids = new Set(commands.map(c => c.pid));
  commands.forEach(c => {
    if (c.parent_pid) {
      if (!cmdChildrenMap[c.parent_pid]) cmdChildrenMap[c.parent_pid] = [];
      cmdChildrenMap[c.parent_pid].push(c);
    }
  });
  // Find roots: parent_pid is not in commands OR parent_pid is null
  const cmdRoots = commands.filter(c => !c.parent_pid || !allPids.has(c.parent_pid));
  // If still empty, just show all commands flat
  const useFlat = cmdRoots.length === 0;
  
  const filtered = q ? commands.filter(c => JSON.stringify(c).toLowerCase().includes(q)) : null;
  
  let html = '<div class="table-controls">';
  html += '<strong>Total: ' + commands.length + '</strong> ';
  html += '<button onclick="window.expandAllCmds()">Expand All</button>';
  html += '<button onclick="window.collapseAllCmds()">Collapse All</button>';
  html += '</div>';
  html += '<table><thead><tr><th>PID</th><th>Parent</th><th>Command</th><th>Start</th><th>End</th><th>Duration</th><th>Exit</th></tr></thead><tbody>';
  
  function renderCmdNode(cmd, depth) {
    const hasChildren = cmdChildrenMap[cmd.pid] && cmdChildrenMap[cmd.pid].length > 0;
    const collapsed = collapsedCmds.has(cmd.pid);
    
    // 使用竖线和分支线条
    const indentLine = depth > 0 ? '<span class="tree-indent">' + '│ '.repeat(depth - 1) + '</span>' : '';
    const branch = depth > 0 ? '<span class="tree-indent">├─</span> ' : '';
    const toggle = hasChildren ? `<span class="tree-toggle" onclick="window.toggleCmd(${cmd.pid})">${collapsed ? '▶' : '─'}</span> ` : '';
    const childCount = hasChildren ? `<span class="subtle">(${cmdChildrenMap[cmd.pid].length})</span>` : '';
    
    const start = cmd.start_unix_millis ? formatUnixMillis(cmd.start_unix_millis) : 'N/A';
    const end = cmd.end_unix_millis ? formatUnixMillis(cmd.end_unix_millis) : 'N/A';
    const dur = cmd.duration_ms !== null && cmd.duration_ms !== undefined ? formatDuration(cmd.duration_ms) : 'N/A';
    
    // Truncate command with hover tooltip
    const cmdText = cmd.command || '';
    const cmdDisplay = cmdText.length > 50 ? cmdText.substring(0, 50) + '...' : cmdText;
    const cmdFull = cmdText.length > 50 ? `data-full="${esc(cmdText)}"` : '';
    
    html += `<tr><td>${indentLine}${branch}${toggle}<strong>${esc(cmd.pid)}</strong> ${childCount}</td><td>${esc(cmd.parent_pid || '-')}</td><td class="cmd-tooltip" ${cmdFull}>${esc(cmdDisplay)}</td><td>${start}</td><td>${end}</td><td class="${getLatencyClass(cmd.duration_ms, max)}">${dur}</td><td>${cmd.exit_code !== null && cmd.exit_code !== undefined ? esc(cmd.exit_code) : '-'}</td></tr>`;
    
    if (hasChildren && !collapsed) {
      cmdChildrenMap[cmd.pid].forEach(child => renderCmdNode(child, depth + 1));
    }
  }
  
  function renderCmdRow(c) {
    const start = c.start_unix_millis ? formatUnixMillis(c.start_unix_millis) : 'N/A';
    const end = c.end_unix_millis ? formatUnixMillis(c.end_unix_millis) : 'N/A';
    const dur = c.duration_ms !== null && c.duration_ms !== undefined ? formatDuration(c.duration_ms) : 'N/A';
    const cmdText = c.command || '';
    const cmdDisplay = cmdText.length > 40 ? cmdText.substring(0, 40) + '...' : cmdText;
    const cmdTooltip = cmdText.length > 40 ? `title="${esc(cmdText)}"` : '';
    html += `<tr><td>${esc(c.pid)}</td><td>${esc(c.parent_pid || '-')}</td><td ${cmdTooltip}>${esc(cmdDisplay)}</td><td>${start}</td><td>${end}</td><td class="${getLatencyClass(c.duration_ms, max)}">${dur}</td><td>${c.exit_code !== null && c.exit_code !== undefined ? esc(c.exit_code) : '-'}</td></tr>`;
  }
  
  if (filtered) {
    filtered.forEach(renderCmdRow);
  } else if (useFlat) {
    commands.forEach(renderCmdRow);
  } else {
    cmdRoots.forEach(root => renderCmdNode(root, 0));
  }
  html += '</tbody></table>';
  $("table").innerHTML = html;
}

window.toggleCmd = function(pid) {
  if (collapsedCmds.has(pid)) {
    collapsedCmds.delete(pid);
  } else {
    collapsedCmds.add(pid);
  }
  renderTable();
};

window.expandAllCmds = function() {
  collapsedCmds.clear();
  renderTable();
};

window.collapseAllCmds = function() {
  const commands = state.data && state.data.analysis && state.data.analysis.commands ? state.data.analysis.commands : [];
  commands.forEach(c => collapsedCmds.add(c.pid));
  renderTable();
};

function renderTimeline(data, q) {
  const timeline = data.timeline || [];
  const filtered = q ? timeline.filter(t => JSON.stringify(t).toLowerCase().includes(q)) : timeline;
  let html = `<table><thead><tr><th>Time</th><th>Lane</th><th>PID</th><th>Type</th><th>Summary</th></tr></thead><tbody>`;
  filtered.forEach(t => {
    html += `<tr><td>${esc(t.time || t.observed_at || '-')}</td><td>${esc(t.lane || '-')}</td><td>${esc(t.pid || '-')}</td><td>${esc(t.type || t.title || '-')}</td><td>${esc(t.summary || '-')}</td></tr>`;
  });
  html += '</tbody></table>';
  $("table").innerHTML = html;
}

function renderEvents(data, q) {
  const list = data.events || [];
  const filtered = q ? list.filter(e => JSON.stringify(e).toLowerCase().includes(q)) : list;
  let html = `<table><thead><tr><th>ID</th><th>Domain</th><th>PID</th><th>Operation</th><th>Summary</th></tr></thead><tbody>`;
  filtered.forEach(e => {
    html += `<tr><td>${esc(e.display_id || e.id)}</td><td>${esc(e.domain)}</td><td>${esc(e.pid)}</td><td>${esc(e.operation || '-')}</td><td>${esc(e.summary || '-')}</td></tr>`;
  });
  html += '</tbody></table>';
  $("table").innerHTML = html;
}

function renderProcessTree(data, q) {
  const tree = data.process_tree || [];
  // Build parent->children map
  const childrenMap = {};
  tree.forEach(p => {
    if (p.parent_pid) {
      if (!childrenMap[p.parent_pid]) childrenMap[p.parent_pid] = [];
      childrenMap[p.parent_pid].push(p);
    }
  });
  // Find roots (no parent)
  const roots = tree.filter(p => !p.parent_pid);
  
  let html = '<div class="table-controls">';
  html += '<button onclick="window.expandAllNodes()">Expand All</button>';
  html += '<button onclick="window.collapseAllNodes()">Collapse All</button>';
  html += '</div>';
  html += '<table><thead><tr><th>PID</th><th>Children</th><th>State</th></tr></thead><tbody>';
  
  function renderNode(node, depth) {
    const hasChildren = childrenMap[node.pid] && childrenMap[node.pid].length > 0;
    const collapsed = collapsedNodes.has(node.pid);
    
    // 使用竖线和分支线条
    const indentLine = depth > 0 ? '<span class="tree-indent">' + '│ '.repeat(depth - 1) + '</span>' : '';
    const branch = depth > 0 ? '<span class="tree-indent">├─</span> ' : '';
    const toggle = hasChildren ? `<span class="tree-toggle" onclick="window.toggleNode(${node.pid})">${collapsed ? '▶' : '─'}</span> ` : '';
    const childCount = hasChildren ? `<span class="subtle">(${childrenMap[node.pid].length})</span>` : '';
    
    html += `<tr><td>${indentLine}${branch}${toggle}<strong>${esc(node.pid)}</strong> ${childCount}</td><td>${esc(node.children || 0)}</td><td>${esc(node.state)}</td></tr>`;
    if (hasChildren && !collapsed) {
      childrenMap[node.pid].forEach(child => renderNode(child, depth + 1));
    }
  }
  
  if (q) {
    const filtered = tree.filter(p => JSON.stringify(p).toLowerCase().includes(q));
    filtered.forEach(p => {
      html += `<tr><td>${'  '.repeat(p.depth || 0)}${esc(p.pid)}</td><td>${esc(p.children || 0)}</td><td>${esc(p.state)}</td></tr>`;
    });
  } else {
    roots.forEach(root => renderNode(root, 0));
  }
  html += '</tbody></table>';
  $("table").innerHTML = html;
}

window.toggleNode = function(pid) {
  if (collapsedNodes.has(pid)) {
    collapsedNodes.delete(pid);
  } else {
    collapsedNodes.add(pid);
  }
  renderTable();
};

window.expandAllNodes = function() {
  collapsedNodes.clear();
  renderTable();
};

window.collapseAllNodes = function() {
  const tree = state.data.process_tree || [];
  tree.forEach(p => collapsedNodes.add(p.pid));
  renderTable();
};

function renderProcesses(data, q) {
  const list = data.processes || [];
  const filtered = q ? list.filter(p => JSON.stringify(p).toLowerCase().includes(q)) : list;
  let html = `<table><thead><tr><th>PID</th><th>Parent</th><th>State</th><th>Exit</th></tr></thead><tbody>`;
  filtered.forEach(p => {
    html += `<tr><td>${esc(p.pid)}</td><td>${esc(p.parent_pid || '-')}</td><td>${esc(p.state)}</td><td>${esc(p.exit_code || '-')}</td></tr>`;
  });
  html += '</tbody></table>';
  $("table").innerHTML = html;
}

function renderNetwork(data, q) {
  const events = data.events || [];
  const net = events.filter(e => e.domain === 'Net' || e.lane === 'net');
  const filtered = q ? net.filter(n => JSON.stringify(n).toLowerCase().includes(q)) : net;
  let html = '<div class="table-controls">';
  html += '<strong>Total: ' + filtered.length + '</strong>';
  html += '</div>';
  html += '<table><thead><tr><th>ID</th><th>PID</th><th>Op</th><th>Local</th><th>Remote</th><th>Size</th></tr></thead><tbody>';
  filtered.forEach(n => {
    // Parse IP from summary (format: "127.0.0.1:48041 -> 127.0.0.53:53")
    const summary = n.summary || '';
    const parts = summary.split(' -> ');
    const local = parts[0] || n.metadata?.local || '-';
    const remote = parts[1] || n.metadata?.remote || '-';
    const size = n.metadata?.requested_size || '-';
    html += `<tr><td>${esc(n.display_id || n.id)}</td><td>${esc(n.pid)}</td><td>${esc(n.operation || '-')}</td><td>${esc(local)}</td><td>${esc(remote)}</td><td>${esc(size)}</td></tr>`;
  });
  html += '</tbody></table>';
  $("table").innerHTML = html;
}

function renderFiles(data, q) {
  const events = data.events || [];
  const files = events.filter(e => e.domain === 'File');
  const filtered = q ? files.filter(f => JSON.stringify(f).toLowerCase().includes(q)) : files;
  let html = `<table><thead><tr><th>ID</th><th>PID</th><th>Op</th><th>Path</th></tr></thead><tbody>`;
  filtered.forEach(f => {
    html += `<tr><td>${esc(f.display_id || f.id)}</td><td>${esc(f.pid)}</td><td>${esc(f.operation || '-')}</td><td style="max-width:300px;overflow:hidden;text-overflow:ellipsis">${esc(f.summary || f.path || '-')}</td></tr>`;
  });
  html += '</tbody></table>';
  $("table").innerHTML = html;
}

function renderPayloads(data, q) {
  const payloads = data.payloads || [];
  const filtered = q ? payloads.filter(p => JSON.stringify(p).toLowerCase().includes(q)) : payloads;
  let html = `<table><thead><tr><th>ID</th><th>PID</th><th>Direction</th><th>Protocol</th><th>Size</th></tr></thead><tbody>`;
  filtered.forEach(p => {
    html += `<tr><td>${esc(p.display_id || p.id)}</td><td>${esc(p.pid)}</td><td>${esc(p.direction || '-')}</td><td>${esc(p.protocol_hint || '-')}</td><td>${esc(p.captured_size || '-')}/${esc(p.original_size || '-')}</td></tr>`;
  });
  html += '</tbody></table>';
  $("table").innerHTML = html;
}

function renderResources(data, q) {
  const events = data.events || [];
  const resources = events.filter(e => e.domain === 'Resource');
  const filtered = q ? resources.filter(r => JSON.stringify(r).toLowerCase().includes(q)) : resources;
  let html = `<table><thead><tr><th>ID</th><th>PID</th><th>Op</th><th>CPU%</th><th>RSS</th></tr></thead><tbody>`;
  filtered.forEach(r => {
    html += `<tr><td>${esc(r.display_id || r.id)}</td><td>${esc(r.pid)}</td><td>${esc(r.operation || '-')}</td><td>${esc(r.cpu_percent || '-')}</td><td>${esc(r.rss_kb || '-')}</td></tr>`;
  });
  html += '</tbody></table>';
  $("table").innerHTML = html;
}

function renderDiagnostics(data, q) {
  const list = data.diagnostics || [];
  const filtered = q ? list.filter(d => JSON.stringify(d).toLowerCase().includes(q)) : list;
  let html = `<table><thead><tr><th>ID</th><th>Severity</th><th>Kind</th><th>Message</th></tr></thead><tbody>`;
  filtered.forEach(d => {
    const sev = d.severity === 'Warning' ? 'warn' : (d.severity === 'Error' ? 'error' : '');
    html += `<tr><td>${esc(d.display_id || d.id || 'diag-' + d.id)}</td><td class="${sev}">${esc(d.severity)}</td><td>${esc(d.kind)}</td><td style="max-width:400px">${esc(d.message)}</td></tr>`;
  });
  html += '</tbody></table>';
  $("table").innerHTML = html;
}

document.querySelectorAll(".tab").forEach(tab => {
  tab.onclick = () => {
    document.querySelectorAll(".tab").forEach(n => n.classList.remove("active"));
    tab.classList.add("active");
    state.tab = tab.dataset.tab;
    renderTable();
  };
});

$("refresh").onclick = loadData;
$("search").oninput = renderTable;
loadData().catch(err => $("detail").textContent = err.message);
"#.to_string()
}