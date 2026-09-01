<template>
  <section
    ref="surface"
    class="fullscreen-surface"
    :class="{
      'fullscreen-surface-fallback': fallbackActive,
      'fullscreen-surface-with-aside': asideOpen,
    }"
  >
    <div class="fullscreen-surface-main">
      <div class="fullscreen-surface-content">
        <slot :fullscreen="isFullscreen" />
      </div>
      <button
        class="fullscreen-surface-toggle icon-button"
        type="button"
        :title="buttonLabel"
        :aria-label="buttonLabel"
        :aria-pressed="isFullscreen"
        @click="toggleFullscreen"
      >
        <Minimize2 v-if="isFullscreen" :size="18" aria-hidden="true" />
        <Maximize2 v-else :size="18" aria-hidden="true" />
      </button>
    </div>
    <div v-if="asideOpen" class="fullscreen-surface-aside">
      <slot name="aside" :fullscreen="isFullscreen" />
    </div>
  </section>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { Maximize2, Minimize2 } from '@lucide/vue';

const props = defineProps({
  label: {
    type: String,
    required: true,
  },
  asideOpen: {
    type: Boolean,
    default: false,
  },
});

const surface = ref(null);
const nativeActive = ref(false);
const fallbackActive = ref(false);
let previousBodyOverflow = '';

const isFullscreen = computed(() => nativeActive.value || fallbackActive.value);
const buttonLabel = computed(() => (
  isFullscreen.value ? `Exit fullscreen: ${props.label}` : `View fullscreen: ${props.label}`
));

onMounted(() => {
  document.addEventListener('fullscreenchange', syncNativeFullscreen);
  document.addEventListener('keydown', exitFallbackOnEscape);
  syncNativeFullscreen();
});

onBeforeUnmount(() => {
  document.removeEventListener('fullscreenchange', syncNativeFullscreen);
  document.removeEventListener('keydown', exitFallbackOnEscape);
  exitFallback();
});

async function toggleFullscreen() {
  if (isFullscreen.value) {
    await exitFullscreen();
    return;
  }
  const target = surface.value;
  if (target?.requestFullscreen) {
    try {
      await target.requestFullscreen();
      return;
    } catch {
      // Some embedded browsers block the native API. Keep the control usable there.
    }
  }
  enterFallback();
}

async function exitFullscreen() {
  if (fallbackActive.value) {
    exitFallback();
    return;
  }
  if (document.fullscreenElement === surface.value && document.exitFullscreen) {
    await document.exitFullscreen();
  }
}

function syncNativeFullscreen() {
  nativeActive.value = document.fullscreenElement === surface.value;
}

function enterFallback() {
  if (fallbackActive.value) {
    return;
  }
  previousBodyOverflow = document.body.style.overflow;
  document.body.style.overflow = 'hidden';
  fallbackActive.value = true;
}

function exitFallback() {
  if (!fallbackActive.value) {
    return;
  }
  fallbackActive.value = false;
  document.body.style.overflow = previousBodyOverflow;
}

function exitFallbackOnEscape(event) {
  if (event.key === 'Escape' && fallbackActive.value) {
    exitFallback();
  }
}
</script>

<style scoped>
.fullscreen-surface {
  position: relative;
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  grid-template-rows: minmax(0, 1fr);
  min-width: 0;
  min-height: 0;
  height: 100%;
  overflow: hidden;
}

.fullscreen-surface-with-aside {
  display: grid;
  grid-template-columns: minmax(420px, 1fr) minmax(320px, 0.42fr);
}

.fullscreen-surface-main {
  position: relative;
  min-width: 0;
  min-height: 0;
  height: 100%;
  overflow: hidden;
}

.fullscreen-surface-content {
  min-width: 0;
  min-height: 0;
  width: 100%;
  height: 100%;
  overflow: auto;
}

.fullscreen-surface-aside {
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.fullscreen-surface-toggle {
  position: absolute;
  z-index: 5;
  top: 14px;
  right: 14px;
  width: 36px;
  height: 36px;
  background: var(--surface);
  box-shadow: 0 4px 16px rgb(0 0 0 / 0.14);
}

.fullscreen-surface:fullscreen,
.fullscreen-surface-fallback {
  width: 100vw;
  height: 100vh;
  background: var(--bg);
}

.fullscreen-surface-fallback {
  position: fixed;
  z-index: 1000;
  inset: 0;
}

@media (max-width: 1000px) {
  .fullscreen-surface-with-aside {
    grid-template-columns: minmax(0, 1fr);
  }

  .fullscreen-surface-aside {
    position: absolute;
    z-index: 6;
    inset: 0 0 0 auto;
    width: min(420px, 80vw);
    box-shadow: -12px 0 30px rgb(0 0 0 / 0.22);
  }
}
</style>
