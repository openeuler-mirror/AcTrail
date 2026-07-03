<template>
  <section class="settings-page">
    <div class="settings-panel">
      <h3>{{ t('stats.llm.settings.pricing') }}</h3>
      <p>{{ t('stats.llm.settings.pricingInactive') }}</p>
      <dl>
        <div>
          <dt>{{ t('stats.llm.settings.estimatedSpend') }}</dt>
          <dd>{{ pricingEnabled ? t('stats.llm.common.available') : t('stats.llm.common.unavailable') }}</dd>
        </div>
        <div>
          <dt>{{ t('stats.llm.settings.currency') }}</dt>
          <dd>CNY</dd>
        </div>
      </dl>
    </div>
  </section>
</template>

<script setup>
import { useLocale } from '../../../locale';

defineProps({
  pricingEnabled: {
    type: Boolean,
    default: false,
  },
});

const { t } = useLocale();
</script>

<style scoped>
.settings-page {
  min-width: 0;
}

.settings-panel {
  max-width: 520px;
  padding: var(--stats-panel-padding);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface-strong);
}

h3 {
  margin: 0 0 var(--stats-space-sm);
  font-size: var(--stats-font-title);
}

p {
  margin: 0 0 var(--stats-space-lg);
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
}

dl {
  display: grid;
  gap: var(--stats-space-sm);
  margin: 0;
}

dl div {
  display: flex;
  justify-content: space-between;
  gap: var(--stats-space-md);
  padding-top: var(--stats-space-sm);
  border-top: 1px solid var(--stats-border);
}

dt {
  color: var(--stats-muted);
}

dd {
  margin: 0;
}
</style>
