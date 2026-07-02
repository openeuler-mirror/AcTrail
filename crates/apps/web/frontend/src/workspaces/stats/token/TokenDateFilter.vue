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
  gap: var(--stats-space-lg);
}

.filter-group label {
  display: grid;
  gap: var(--stats-space-xs);
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
  font-weight: var(--stats-weight-medium);
  text-transform: uppercase;
}

.filter-group input {
  width: var(--stats-date-input-width);
  height: var(--stats-control-height-lg);
  padding: 0 var(--stats-space-lg);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-md);
  background: var(--stats-surface-strong);
  color: var(--stats-text);
  outline: 0;
}

.filter-group input:focus {
  border-color: transparent;
  box-shadow:
    0 0 0 2px var(--stats-accent),
    0 0 0 4px var(--stats-bg-base);
}
</style>
