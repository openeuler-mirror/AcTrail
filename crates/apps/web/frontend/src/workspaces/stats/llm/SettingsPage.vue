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
  padding: var(--ui-panel-padding);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface-strong);
}

h3 {
  margin: 0 0 var(--ui-space-sm);
  font-size: var(--ui-font-title);
}

p {
  margin: 0 0 var(--ui-space-lg);
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
}

dl {
  display: grid;
  gap: var(--ui-space-sm);
  margin: 0;
}

dl div {
  display: flex;
  justify-content: space-between;
  gap: var(--ui-space-md);
  padding-top: var(--ui-space-sm);
  border-top: 1px solid var(--ui-border);
}

dt {
  color: var(--ui-muted);
}

dd {
  margin: 0;
}
</style>
