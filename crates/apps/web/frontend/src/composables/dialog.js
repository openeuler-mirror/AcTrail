/**
 * Shared dialog behaviour: focus entry, focus trap, focus restore, Escape to
 * close and a scroll lock on the workspace scroll container.
 */
import { nextTick, onBeforeUnmount, ref, watch } from 'vue';

const FOCUSABLE = [
  'a[href]',
  'button:not([disabled])',
  'input:not([disabled])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
].join(',');

/**
 * @param {object} options
 * @param {() => boolean} options.isOpen - Whether the dialog is mounted.
 * @param {() => HTMLElement | null} options.getContainer - Dialog element getter.
 * @param {() => void} options.onClose - Close handler for Escape.
 * @param {object} [options.initialFocus] - Ref holding the element to focus first.
 */
export function useDialogBehavior({ isOpen, getContainer, onClose, initialFocus }) {
  const restoreTarget = ref(null);

  watch(isOpen, async (open, wasOpen) => {
    if (open) {
      restoreTarget.value = document.activeElement;
      document.documentElement.dataset.dialogOpen = 'true';
      await nextTick();
      const preferred = initialFocus?.value ?? findFocusable(getContainer())?.[0];
      preferred?.focus?.();
      return;
    }
    if (wasOpen) {
      delete document.documentElement.dataset.dialogOpen;
      restoreTarget.value?.focus?.();
      restoreTarget.value = null;
    }
  }, { immediate: true });

  function onKeydown(event) {
    if (!isOpen()) {
      return;
    }
    if (event.key === 'Escape') {
      event.stopPropagation();
      onClose();
      return;
    }
    if (event.key !== 'Tab') {
      return;
    }
    const container = getContainer();
    const focusable = findFocusable(container);
    if (!focusable.length) {
      return;
    }
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    const active = document.activeElement;
    if (event.shiftKey && (active === first || !container?.contains(active))) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && active === last) {
      event.preventDefault();
      first.focus();
    }
  }

  window.addEventListener('keydown', onKeydown);
  onBeforeUnmount(() => {
    window.removeEventListener('keydown', onKeydown);
    delete document.documentElement.dataset.dialogOpen;
  });
}

function findFocusable(container) {
  if (!container) {
    return [];
  }
  return [...container.querySelectorAll(FOCUSABLE)].filter(
    (element) => element.offsetParent !== null || element === document.activeElement,
  );
}
