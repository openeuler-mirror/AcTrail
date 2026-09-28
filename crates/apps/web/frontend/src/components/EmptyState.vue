<template>
  <div class="empty-state" :class="{ 'empty-state-compact': compact }">
    <span class="empty-icon" aria-hidden="true">
      <slot name="icon"><Inbox :size="18" /></slot>
    </span>
    <div class="empty-copy">
      <strong>{{ title }}</strong>
      <p v-if="description">{{ description }}</p>
    </div>
    <div v-if="$slots.action" class="empty-action">
      <slot name="action" />
    </div>
  </div>
</template>

<script setup>
import { Inbox } from '@lucide/vue';

defineProps({
  title: { type: String, required: true },
  description: { type: String, default: '' },
  /** Compact drops the frame, for empty states inside an existing panel. */
  compact: { type: Boolean, default: false },
});
</script>

<style scoped>
.empty-state {
  min-width: 0;
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--ui-space-md);
  padding: var(--ui-space-lg) var(--ui-space-xl);
  border: 1px dashed color-mix(in srgb, var(--ui-border) 90%, transparent);
  border-radius: var(--ui-radius-lg);
  background: color-mix(in srgb, var(--ui-surface) 60%, transparent);
  text-align: left;
}

.empty-state-compact {
  padding: var(--ui-space-sm) 0;
  border: 0;
  background: transparent;
}

.empty-icon {
  width: 42px;
  height: 42px;
  display: grid;
  place-items: center;
  border: 2px solid color-mix(in srgb, var(--ui-border-strong) 80%, transparent);
  border-radius: 50%;
  color: var(--ui-muted);
}

.empty-state-compact .empty-icon {
  width: 28px;
  height: 28px;
  border-width: 1px;
}

.empty-copy {
  min-width: 0;
  display: grid;
  gap: 2px;
}

.empty-copy strong {
  color: var(--ui-text);
  font-size: var(--ui-font-md);
  font-weight: var(--ui-weight-semibold);
}

.empty-copy p {
  max-width: 72ch;
  margin: 0;
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
  line-height: 1.5;
}

.empty-action {
  display: inline-flex;
  gap: var(--ui-space-xs);
}

@media (max-width: 760px) {
  .empty-state {
    grid-template-columns: auto minmax(0, 1fr);
  }

  .empty-action {
    grid-column: 2;
    justify-self: start;
  }
}
</style>
