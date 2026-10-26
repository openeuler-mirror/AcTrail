/**
 * Scroll reveal directive: elements rise and fade in the first time they
 * intersect the viewport, staggered by the binding value. The animation is a
 * pure CSS transition so nothing runs per frame, and reduced motion collapses
 * it to an immediate show.
 */
const REVEAL_SELECTOR = '[data-reveal]';
const observed = new WeakMap();

function reveal(element) {
  element.dataset.revealState = 'visible';
}

export const vReveal = {
  mounted(element, binding) {
    if (typeof IntersectionObserver === 'undefined') {
      reveal(element);
      return;
    }
    element.dataset.reveal = '';
    element.dataset.revealState = 'hidden';
    if (binding.value) {
      element.style.setProperty('--reveal-delay', `${Number(binding.value) * 70}ms`);
    }
    const observer = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) {
          continue;
        }
        reveal(entry.target);
        observer.unobserve(entry.target);
      }
    }, { rootMargin: '0px 0px -8% 0px', threshold: 0.05 });
    observer.observe(element);
    observed.set(element, observer);
    // Never leave content hidden if the observer cannot report for this node
    // (zero-height container at mount, or a detached subtree).
    observed.set(element, {
      disconnect() {
        observer.disconnect();
        window.clearTimeout(safetyTimer);
      },
    });
    const safetyTimer = window.setTimeout(() => reveal(element), 1200);
  },
  unmounted(element) {
    observed.get(element)?.disconnect();
    observed.delete(element);
  },
};

export const REVEAL_STYLE = `
${REVEAL_SELECTOR}[data-reveal-state='hidden'] {
  opacity: 0;
  transform: translateY(10px);
}

${REVEAL_SELECTOR} {
  transition:
    opacity 360ms var(--ui-ease-out, ease-out),
    transform 360ms var(--ui-ease-out, ease-out);
  transition-delay: var(--reveal-delay, 0ms);
}

@media (prefers-reduced-motion: reduce) {
  ${REVEAL_SELECTOR} {
    transition: none;
  }
}
`;
