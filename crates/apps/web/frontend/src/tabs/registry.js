import EventsTab from './activity/events/EventsTab.vue';
import FilesTab from './activity/files/FilesTab.vue';
import NetworkTab from './activity/network/NetworkTab.vue';
import PayloadsTab from './activity/payloads/PayloadsTab.vue';
import ActionTreeTab from './core/action-tree/ActionTreeTab.vue';
import CommandsTab from './core/commands/CommandsTab.vue';
import OverviewTab from './core/overview/OverviewTab.vue';
import TimelineTab from './core/timeline/TimelineTab.vue';
import WaterfallTab from './core/waterfall/WaterfallTab.vue';
import AlertsTab from './system/alerts/AlertsTab.vue';
import DiagnosticsTab from './system/diagnostics/DiagnosticsTab.vue';
import ProcessesTab from './system/processes/ProcessesTab.vue';
import ProcessTreeTab from './system/process-tree/ProcessTreeTab.vue';
import ResourcesTab from './system/resources/ResourcesTab.vue';

export const TAB_IDS = Object.freeze({
  overview: 'overview',
  actionTree: 'action_tree',
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

export const TAB_DEFINITIONS = Object.freeze([
  { id: TAB_IDS.overview, label: 'Overview', component: OverviewTab },
  { id: TAB_IDS.actionTree, label: 'Action Tree', component: ActionTreeTab },
  { id: TAB_IDS.waterfall, label: 'Waterfall', component: WaterfallTab },
  { id: TAB_IDS.commands, label: 'Commands', component: CommandsTab },
  { id: TAB_IDS.timeline, label: 'Timeline', component: TimelineTab },
  { id: TAB_IDS.events, label: 'Events', component: EventsTab },
  { id: TAB_IDS.processTree, label: 'Process Tree', component: ProcessTreeTab },
  { id: TAB_IDS.processes, label: 'Processes', component: ProcessesTab },
  { id: TAB_IDS.network, label: 'Network', component: NetworkTab },
  { id: TAB_IDS.files, label: 'Files', component: FilesTab },
  { id: TAB_IDS.payloads, label: 'Payloads', component: PayloadsTab },
  { id: TAB_IDS.resources, label: 'Resources', component: ResourcesTab },
  { id: TAB_IDS.alerts, label: 'Alerts', component: AlertsTab },
  { id: TAB_IDS.diagnostics, label: 'Diagnostics', component: DiagnosticsTab },
]);
