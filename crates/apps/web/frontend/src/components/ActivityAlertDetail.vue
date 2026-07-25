<template>
  <div class="activity-alert-detail">
    <section class="activity-rule">
      <header>
        <div>
          <span>{{ t('alerts.activity.ruleKicker') }}</span>
          <h3>{{ ruleTitle }}</h3>
        </div>
        <strong>{{ t('alerts.activity.findingCount', { count: findings.length }) }}</strong>
      </header>

      <div v-if="isCommand" class="activity-rule-copy">
        <p>
          {{ t('alerts.activity.commandRulePrefix') }}
          <strong>{{ formatDuration(payload.maximum_duration_ms) }}</strong>
        </p>
      </div>
      <div v-else class="activity-rule-copy">
        <p>{{ t('alerts.activity.growthRulePrefix') }}</p>
        <ul>
          <li>
            {{ t('alerts.activity.hardLimitRule') }}
            <strong>{{ formatBytes(payload.hard_limit_bytes) }}</strong>
          </li>
          <li>
            {{
              t('alerts.activity.relativeGrowthRule', {
                samples: formatInteger(payload.minimum_samples),
                current: formatBytes(payload.minimum_current_bytes),
                growth: formatBytes(payload.minimum_growth_bytes),
                ratio: formatRatio(payload.ratio_per_mille),
                window: formatInteger(payload.window_size),
              })
            }}
          </li>
        </ul>
      </div>
    </section>

    <section class="activity-findings">
      <header class="activity-section-heading">
        <div>
          <span>{{ t('alerts.activity.evidenceKicker') }}</span>
          <h3>{{ t('alerts.activity.evidenceTitle') }}</h3>
        </div>
        <span v-if="truncatedCount" class="activity-truncated">
          {{ t('alerts.activity.truncated', { count: formatInteger(truncatedCount) }) }}
        </span>
      </header>

      <article
        v-for="(finding, index) in findings"
        :key="finding.action_id || index"
        class="activity-finding"
      >
        <template v-if="isCommand">
          <header class="activity-finding-heading">
            <div>
              <span>{{ t('alerts.activity.command') }}</span>
              <code>{{ commandText(finding) }}</code>
            </div>
            <span class="activity-reason">{{ statusLabel(finding.status) }}</span>
          </header>
          <p v-if="!finding.command_line" class="activity-data-note">
            {{ t('alerts.activity.commandLineUnavailable') }}
          </p>

          <dl class="activity-metrics activity-metrics-command">
            <div class="activity-metric-primary">
              <dt>{{ t('alerts.activity.actualDuration') }}</dt>
              <dd>{{ formatDuration(finding.duration_ms) }}</dd>
            </div>
            <div>
              <dt>{{ t('alerts.activity.durationThreshold') }}</dt>
              <dd>{{ formatDuration(payload.maximum_duration_ms) }}</dd>
            </div>
            <div>
              <dt>{{ t('alerts.activity.exceededBy') }}</dt>
              <dd>{{ formatDuration(exceededDuration(finding)) }}</dd>
            </div>
          </dl>

          <dl class="activity-evidence-fields">
            <div><dt>{{ t('alerts.activity.startedAt') }}</dt><dd>{{ formatTime(finding.started_at_ms) }}</dd></div>
            <div><dt>{{ t('alerts.activity.endedAt') }}</dt><dd>{{ formatTime(finding.ended_at_ms) }}</dd></div>
            <div v-if="finding.executable && finding.executable !== finding.command_line">
              <dt>{{ t('alerts.activity.executable') }}</dt>
              <dd><code>{{ finding.executable }}</code></dd>
            </div>
            <div><dt>{{ t('alerts.activity.exitCode') }}</dt><dd>{{ valueOrDash(finding.exit_code) }}</dd></div>
            <div><dt>{{ t('alerts.activity.processId') }}</dt><dd>{{ finding.process_id }}</dd></div>
            <div><dt>{{ t('alerts.activity.actionId') }}</dt><dd>{{ finding.action_id }}</dd></div>
            <div v-if="finding.agent_action_id">
              <dt>{{ t('alerts.activity.agentActionId') }}</dt>
              <dd>{{ finding.agent_action_id }}</dd>
            </div>
          </dl>
        </template>

        <template v-else>
          <header class="activity-finding-heading">
            <div>
              <span>{{ directionLabel }}</span>
              <strong>{{ formatBytes(finding.observed_bytes) }}</strong>
            </div>
            <span class="activity-reason">{{ reasonLabel(finding.reason) }}</span>
          </header>

          <p class="activity-trigger-summary">{{ triggerSummary(finding) }}</p>

          <dl class="activity-metrics">
            <div class="activity-metric-primary">
              <dt>{{ t('alerts.activity.observedSize') }}</dt>
              <dd>{{ formatBytes(finding.observed_bytes) }}</dd>
            </div>
            <div>
              <dt>{{ t('alerts.activity.hardLimit') }}</dt>
              <dd>{{ formatBytes(payload.hard_limit_bytes) }}</dd>
            </div>
            <div>
              <dt>{{ t('alerts.activity.baselineMedian') }}</dt>
              <dd>{{ nullableBytes(finding.baseline_median_bytes) }}</dd>
            </div>
            <div>
              <dt>{{ t('alerts.activity.observedRatio') }}</dt>
              <dd>{{ nullableRatio(finding.observed_ratio_per_mille) }}</dd>
            </div>
          </dl>

          <dl class="activity-evidence-fields">
            <div><dt>{{ t('alerts.activity.requestTime') }}</dt><dd>{{ formatTime(finding.started_at_ms) }}</dd></div>
            <div><dt>{{ t('alerts.activity.model') }}</dt><dd>{{ valueOrDash(finding.model) }}</dd></div>
            <div><dt>{{ t('alerts.activity.server') }}</dt><dd>{{ serverTarget(finding) }}</dd></div>
            <div><dt>{{ t('alerts.activity.processId') }}</dt><dd>{{ finding.process_id }}</dd></div>
            <div><dt>{{ t('alerts.activity.actionId') }}</dt><dd>{{ finding.action_id }}</dd></div>
            <div><dt>{{ t('alerts.activity.callActionId') }}</dt><dd>{{ finding.call_action_id }}</dd></div>
          </dl>
        </template>
      </article>
    </section>

    <section class="activity-context">
      <header class="activity-section-heading">
        <div>
          <span>{{ t('alerts.activity.contextKicker') }}</span>
          <h3>{{ t('alerts.activity.contextTitle') }}</h3>
        </div>
      </header>
      <dl class="activity-evidence-fields">
        <div><dt>{{ t('alerts.activity.traceName') }}</dt><dd>{{ valueOrDash(payload.display_name) }}</dd></div>
        <div><dt>{{ t('alerts.activity.profile') }}</dt><dd>{{ valueOrDash(payload.profile_name) }}</dd></div>
        <div><dt>{{ t('alerts.activity.rootProcess') }}</dt><dd>{{ valueOrDash(payload.root_process_id) }}</dd></div>
        <div><dt>{{ t('alerts.activity.container') }}</dt><dd>{{ valueOrDash(payload.root_container_id) }}</dd></div>
      </dl>
    </section>
  </div>
</template>

<script setup>
import { computed } from 'vue';

import { useLocale } from '../locale';

const props = defineProps({
  alert: {
    type: Object,
    required: true,
  },
});

const { currentLanguage, t } = useLocale();
const payload = computed(() => props.alert?.payload ?? {});
const findings = computed(() =>
  Array.isArray(payload.value.findings) ? payload.value.findings : [],
);
const truncatedCount = computed(() => Number(payload.value.truncated_count) || 0);
const isCommand = computed(() => props.alert?.kind === 'command.duration.exceeded');
const directionLabel = computed(() =>
  payload.value.direction === 'response'
    ? t('alerts.activity.response')
    : t('alerts.activity.request'),
);
const ruleTitle = computed(() =>
  isCommand.value
    ? t('alerts.activity.commandRuleTitle')
    : t('alerts.activity.growthRuleTitle', { direction: directionLabel.value }),
);

function commandText(finding) {
  return finding.command_line || finding.executable || t('alerts.activity.unknownCommand');
}

function exceededDuration(finding) {
  const actual = Number(finding.duration_ms);
  const threshold = Number(payload.value.maximum_duration_ms);
  return Number.isFinite(actual) && Number.isFinite(threshold)
    ? Math.max(0, actual - threshold)
    : null;
}

function triggerSummary(finding) {
  if (finding.reason === 'relative-growth') {
    return t('alerts.activity.relativeGrowthSummary', {
      observed: formatBytes(finding.observed_bytes),
      baseline: nullableBytes(finding.baseline_median_bytes),
      ratio: nullableRatio(finding.observed_ratio_per_mille),
      threshold: formatRatio(payload.value.ratio_per_mille),
    });
  }
  return t('alerts.activity.hardLimitSummary', {
    observed: formatBytes(finding.observed_bytes),
    threshold: formatBytes(payload.value.hard_limit_bytes),
  });
}

function reasonLabel(reason) {
  return reason === 'relative-growth'
    ? t('alerts.activity.relativeGrowth')
    : t('alerts.activity.hardLimitReached');
}

function statusLabel(status) {
  const labels = {
    success: t('alerts.activity.statusSuccess'),
    failed: t('alerts.activity.statusFailed'),
    'in-progress': t('alerts.activity.statusInProgress'),
  };
  return labels[status] ?? valueOrDash(status);
}

function serverTarget(finding) {
  if (!finding.server_address && !finding.url_path) {
    return '—';
  }
  return `${finding.server_address ?? ''}${finding.url_path ?? ''}`;
}

function nullableBytes(value) {
  return value == null ? '—' : formatBytes(value);
}

function nullableRatio(value) {
  return value == null ? '—' : formatRatio(value);
}

function valueOrDash(value) {
  return value == null || value === '' ? '—' : String(value);
}

function formatInteger(value) {
  if (value == null || value === '') return '—';
  const number = Number(value);
  if (!Number.isFinite(number)) return '—';
  return new Intl.NumberFormat(currentLanguage.value, { maximumFractionDigits: 0 }).format(number);
}

function formatDecimal(value, maximumFractionDigits = 2) {
  if (value == null || value === '') return '—';
  const number = Number(value);
  if (!Number.isFinite(number)) return '—';
  return new Intl.NumberFormat(currentLanguage.value, { maximumFractionDigits }).format(number);
}

function formatBytes(value) {
  if (value == null || value === '') return '—';
  const bytes = Number(value);
  if (!Number.isFinite(bytes)) return '—';
  if (Math.abs(bytes) < 1024) return `${formatInteger(bytes)} B`;
  const units = ['KiB', 'MiB', 'GiB', 'TiB'];
  let scaled = bytes;
  let unitIndex = -1;
  do {
    scaled /= 1024;
    unitIndex += 1;
  } while (Math.abs(scaled) >= 1024 && unitIndex < units.length - 1);
  return `${formatDecimal(scaled)} ${units[unitIndex]} (${formatInteger(bytes)} B)`;
}

function formatRatio(perMille) {
  if (perMille == null || perMille === '') return '—';
  const value = Number(perMille);
  return Number.isFinite(value) ? `${formatDecimal(value / 1000)}×` : '—';
}

function formatDuration(value) {
  if (value == null || value === '') return '—';
  const milliseconds = Number(value);
  if (!Number.isFinite(milliseconds)) return '—';
  if (milliseconds < 1000) return `${formatInteger(milliseconds)} ms`;
  if (milliseconds < 60000) {
    return `${formatDecimal(milliseconds / 1000)} s (${formatInteger(milliseconds)} ms)`;
  }
  return `${formatDecimal(milliseconds / 60000)} min (${formatDecimal(milliseconds / 1000)} s)`;
}

function formatTime(timestamp) {
  const value = Number(timestamp);
  if (!Number.isFinite(value)) return valueOrDash(timestamp);
  return new Date(value).toLocaleString(currentLanguage.value, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    timeZoneName: 'short',
  });
}
</script>

<style scoped>
.activity-alert-detail {
  display: grid;
  gap: var(--stats-space-xl);
}

.activity-rule,
.activity-findings,
.activity-context {
  min-width: 0;
  overflow: hidden;
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-md);
  background: var(--stats-surface-soft);
}

.activity-rule > header,
.activity-section-heading,
.activity-finding-heading {
  min-width: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--stats-space-lg);
}

.activity-rule > header,
.activity-section-heading {
  padding: var(--stats-space-lg) var(--stats-space-xl);
  border-bottom: 1px solid var(--stats-border);
  background: var(--stats-surface-bar);
}

.activity-rule header span,
.activity-section-heading span,
.activity-finding-heading span,
.activity-evidence-fields dt,
.activity-metrics dt {
  color: var(--stats-muted);
  font-size: var(--stats-font-xs);
  font-weight: var(--stats-weight-medium);
  text-transform: uppercase;
}

.activity-rule h3,
.activity-section-heading h3 {
  margin: var(--stats-space-2xs) 0 0;
  color: var(--stats-text);
  font-family: var(--stats-heading-font);
  font-size: var(--stats-font-display-sm);
  font-weight: var(--stats-weight-medium);
}

.activity-rule header strong {
  flex: 0 0 auto;
  color: var(--stats-accent);
  font-family: var(--stats-value-font);
  font-size: var(--stats-font-sm);
}

.activity-rule-copy {
  padding: var(--stats-space-lg) var(--stats-space-xl);
  color: var(--stats-text);
  line-height: 1.65;
}

.activity-rule-copy p {
  margin: 0;
}

.activity-rule-copy ul {
  display: grid;
  gap: var(--stats-space-sm);
  margin: var(--stats-space-sm) 0 0;
  padding-left: var(--stats-space-xl);
}

.activity-rule-copy strong {
  color: var(--stats-danger);
  font-family: var(--stats-value-font);
}

.activity-findings {
  display: grid;
}

.activity-truncated {
  color: var(--stats-warning) !important;
  text-transform: none !important;
}

.activity-finding {
  min-width: 0;
  display: grid;
  gap: var(--stats-space-lg);
  padding: var(--stats-space-xl);
  border-bottom: 1px solid var(--stats-border);
}

.activity-finding:last-child {
  border-bottom: 0;
}

.activity-finding-heading {
  align-items: flex-start;
}

.activity-finding-heading > div {
  min-width: 0;
  display: grid;
  gap: var(--stats-space-xs);
}

.activity-finding-heading code,
.activity-finding-heading strong {
  color: var(--stats-text);
  font-family: var(--stats-value-font);
  font-size: var(--stats-font-display-sm);
  font-weight: var(--stats-weight-medium);
  line-height: 1.45;
  overflow-wrap: anywhere;
  white-space: pre-wrap;
}

.activity-reason {
  flex: 0 0 auto;
  padding: var(--stats-space-xs) var(--stats-space-sm);
  border: 1px solid color-mix(in srgb, var(--stats-warning) 36%, transparent);
  border-radius: var(--stats-radius-sm);
  background: color-mix(in srgb, var(--stats-warning) 10%, transparent);
  color: var(--stats-warning) !important;
  text-transform: none !important;
}

.activity-trigger-summary {
  margin: 0;
  padding: var(--stats-space-sm) var(--stats-space-md);
  border-left: 3px solid var(--stats-warning);
  background: color-mix(in srgb, var(--stats-warning) 7%, transparent);
  color: var(--stats-text);
  font-size: var(--stats-font-sm);
  line-height: 1.6;
}

.activity-data-note {
  margin: 0;
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
  line-height: 1.55;
}

.activity-metrics {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(10rem, 100%), 1fr));
  gap: var(--stats-space-sm);
  margin: 0;
}

.activity-metrics div {
  min-width: 0;
  padding: var(--stats-space-md);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface);
}

.activity-metrics dd {
  margin: var(--stats-space-xs) 0 0;
  color: var(--stats-text);
  font-family: var(--stats-value-font);
  font-size: var(--stats-font-md);
  font-weight: var(--stats-weight-medium);
  overflow-wrap: anywhere;
}

.activity-metrics .activity-metric-primary {
  border-color: color-mix(in srgb, var(--stats-danger) 32%, var(--stats-border));
  background: color-mix(in srgb, var(--stats-danger) 7%, var(--stats-surface));
}

.activity-metric-primary dd {
  color: var(--stats-danger);
}

.activity-evidence-fields {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(13rem, 100%), 1fr));
  gap: var(--stats-space-md) var(--stats-space-xl);
  margin: 0;
}

.activity-evidence-fields div {
  min-width: 0;
}

.activity-evidence-fields dd {
  margin: var(--stats-space-xs) 0 0;
  color: var(--stats-text);
  font-family: var(--stats-value-font);
  font-size: var(--stats-font-sm);
  line-height: 1.5;
  overflow-wrap: anywhere;
}

.activity-context .activity-evidence-fields {
  padding: var(--stats-space-xl);
}

@media (max-width: 47.5rem) {
  .activity-rule > header,
  .activity-section-heading,
  .activity-finding-heading {
    align-items: flex-start;
    flex-direction: column;
  }

  .activity-finding {
    padding: var(--stats-space-lg);
  }
}
</style>
