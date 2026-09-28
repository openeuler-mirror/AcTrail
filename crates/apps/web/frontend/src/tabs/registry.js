import EventsTab from './activity/events/EventsTab.vue';
import FilesTab from './activity/files/FilesTab.vue';
import NetworkTab from './activity/network/NetworkTab.vue';
import PayloadsTab from './activity/payloads/PayloadsTab.vue';
import ActionTreeTab from './core/action-tree/ActionTreeTab.vue';
import CommandsTab from './core/commands/CommandsTab.vue';
import FlameGraphTab from './core/flame-graph/FlameGraphTab.vue';
import LlmTrajectoryTab from './core/llm-trajectory/LlmTrajectoryTab.vue';
import TimelineTab from './core/timeline/TimelineTab.vue';
import WaterfallTab from './core/waterfall/WaterfallTab.vue';
import AlertsTab from './system/alerts/AlertsTab.vue';
import DiagnosticsTab from './system/diagnostics/DiagnosticsTab.vue';
import ProcessesTab from './system/processes/ProcessesTab.vue';
import ProcessTreeTab from './system/process-tree/ProcessTreeTab.vue';
import ResourcesTab from './system/resources/ResourcesTab.vue';

export const TAB_IDS = Object.freeze({
  actionTree: 'action_tree',
  flameGraph: 'flame_graph',
  llmTrajectory: 'llm_trajectory',
  waterfall: 'waterfall',
  commands: 'commands',
  timeline: 'timeline',
  events: 'events',
  processTree: 'process_tree',
  processes: 'processes',
  network: 'network',
  files: 'files',
  payloads: 'payloads',
  resources: 'resources',
  diagnostics: 'diagnostics',
  alerts: 'alerts',
});

export const TAB_GROUP_IDS = Object.freeze({
  execution: 'execution',
  activity: 'activity',
  health: 'health',
});

export const TAB_DEFINITIONS = Object.freeze([
  { id: TAB_IDS.actionTree, labelKey: 'views.actionTree', component: ActionTreeTab },
  { id: TAB_IDS.llmTrajectory, labelKey: 'views.llmTrajectory', component: LlmTrajectoryTab },
  { id: TAB_IDS.flameGraph, labelKey: 'views.flameGraph', component: FlameGraphTab },
  { id: TAB_IDS.waterfall, labelKey: 'views.waterfall', component: WaterfallTab },
  { id: TAB_IDS.commands, labelKey: 'views.commands', component: CommandsTab },
  { id: TAB_IDS.timeline, labelKey: 'views.timeline', component: TimelineTab },
  { id: TAB_IDS.events, labelKey: 'views.events', component: EventsTab },
  { id: TAB_IDS.processTree, labelKey: 'views.processTree', component: ProcessTreeTab },
  { id: TAB_IDS.processes, labelKey: 'views.processes', component: ProcessesTab },
  { id: TAB_IDS.network, labelKey: 'views.network', component: NetworkTab },
  { id: TAB_IDS.files, labelKey: 'views.files', component: FilesTab },
  { id: TAB_IDS.payloads, labelKey: 'views.payloads', component: PayloadsTab },
  { id: TAB_IDS.resources, labelKey: 'views.resources', component: ResourcesTab },
  { id: TAB_IDS.alerts, labelKey: 'views.alerts', component: AlertsTab },
  { id: TAB_IDS.diagnostics, labelKey: 'views.diagnostics', component: DiagnosticsTab },
]);

export const TAB_GROUP_DEFINITIONS = Object.freeze([
  {
    id: TAB_GROUP_IDS.execution,
    labelKey: 'groups.execution',
    defaultTabId: TAB_IDS.flameGraph,
    tabIds: Object.freeze([
      TAB_IDS.flameGraph,
      TAB_IDS.actionTree,
      TAB_IDS.llmTrajectory,
      TAB_IDS.commands,
      TAB_IDS.processes,
      TAB_IDS.processTree,
      TAB_IDS.waterfall,
    ]),
  },
  {
    id: TAB_GROUP_IDS.activity,
    labelKey: 'groups.activity',
    defaultTabId: TAB_IDS.timeline,
    tabIds: Object.freeze([
      TAB_IDS.timeline,
      TAB_IDS.events,
      TAB_IDS.files,
      TAB_IDS.network,
      TAB_IDS.payloads,
      TAB_IDS.resources,
    ]),
  },
  {
    id: TAB_GROUP_IDS.health,
    labelKey: 'groups.health',
    defaultTabId: TAB_IDS.alerts,
    tabIds: Object.freeze([TAB_IDS.alerts, TAB_IDS.diagnostics]),
  },
]);
