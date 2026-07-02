<template>
  <div class="bucket-control" aria-label="Token time bucket">
    <button
      v-for="bucket in buckets"
      :key="bucket.id"
      type="button"
      :class="{ active: modelValue === bucket.id }"
      @click="$emit('update:modelValue', bucket.id)"
    >
      {{ bucket.label }}
    </button>
  </div>
</template>

<script setup>
defineProps({
  buckets: {
    type: Array,
    required: true,
  },
  modelValue: {
    type: String,
    required: true,
  },
});

defineEmits(['update:modelValue']);
</script>

<style scoped>
.bucket-control {
  display: inline-flex;
  min-width: 0;
  flex-wrap: wrap;
  gap: var(--stats-space-2xs);
  padding: var(--stats-space-2xs);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-md);
  background: var(--stats-surface);
  backdrop-filter: var(--stats-control-filter);
}

.bucket-control button {
  height: var(--stats-control-height-sm);
  padding: 0 var(--stats-segment-padding-x);
  border: 0;
  border-radius: var(--stats-radius-sm);
  background: transparent;
  color: var(--stats-muted);
  cursor: pointer;
  font-size: var(--stats-font-sm);
  font-weight: var(--stats-weight-medium);
}

.bucket-control button:hover,
.bucket-control button.active {
  background: var(--stats-accent);
  color: var(--stats-on-accent);
}
</style>
