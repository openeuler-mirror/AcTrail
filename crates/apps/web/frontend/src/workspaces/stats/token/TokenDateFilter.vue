<template>
  <section class="token-filter">
    <div class="filter-group">
      <label>
        <span>From</span>
        <input
          :value="fromDate"
          type="date"
          :disabled="disabled"
          @input="update('fromDate', $event.target.value)"
        />
      </label>
      <label>
        <span>To</span>
        <input
          :value="toDate"
          type="date"
          :disabled="disabled"
          @input="update('toDate', $event.target.value)"
        />
      </label>
    </div>
  </section>
</template>

<script setup>
const props = defineProps({
  fromDate: {
    type: String,
    required: true,
  },
  toDate: {
    type: String,
    required: true,
  },
  disabled: {
    type: Boolean,
    default: false,
  },
});

const emit = defineEmits(['update-range']);

function update(key, value) {
  emit('update-range', {
    fromDate: key === 'fromDate' ? value : props.fromDate,
    toDate: key === 'toDate' ? value : props.toDate,
  });
}
</script>

<style scoped>
.token-filter {
  min-width: 0;
}

.filter-group {
  display: flex;
  flex-wrap: wrap;
  gap: var(--ui-space-lg);
}

.filter-group label {
  display: grid;
  gap: var(--ui-space-xs);
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
  font-weight: var(--ui-weight-medium);
  text-transform: uppercase;
}

.filter-group input {
  width: var(--ui-date-input-width);
  height: var(--ui-control-height-lg);
  padding: 0 var(--ui-space-lg);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-md);
  background: var(--ui-surface-strong);
  color: var(--ui-text);
  outline: 0;
}

.filter-group input:focus {
  border-color: transparent;
  box-shadow:
    0 0 0 2px var(--ui-accent),
    0 0 0 4px var(--ui-bg-base);
}
</style>
