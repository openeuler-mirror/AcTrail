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
  gap: var(--ui-space-2xs);
  padding: var(--ui-space-2xs);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-md);
  background: var(--ui-surface);
  backdrop-filter: var(--ui-control-filter);
}

.bucket-control button {
  height: var(--ui-control-height-sm);
  padding: 0 var(--ui-segment-padding-x);
  border: 0;
  border-radius: var(--ui-radius-sm);
  background: transparent;
  color: var(--ui-muted);
  cursor: pointer;
  font-size: var(--ui-font-sm);
  font-weight: var(--ui-weight-medium);
}

.bucket-control button:hover,
.bucket-control button.active {
  background: var(--ui-accent);
  color: var(--ui-on-accent);
}
</style>
