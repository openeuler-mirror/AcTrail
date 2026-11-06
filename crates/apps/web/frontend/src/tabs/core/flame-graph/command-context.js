/** Visible command IDs emphasized by the current focus. */
export class CommandContext {
  constructor(entries, focusId) {
    this.related = new Set();
    if (focusId == null) return;
    const focus = entries.find(({ activity }) => activity.id === focusId
      && activity.commandHierarchy && !activity.density && !activity.synthetic);
    if (!focus) return;
    this.related.add(focusId);
    const parentId = focus.activity.commandHierarchy.parentId;
    for (const { activity } of entries) {
      if (!activity.commandHierarchy || activity.density || activity.synthetic) continue;
      if (activity.id === parentId || activity.commandHierarchy.parentId === focusId) {
        this.related.add(activity.id);
      }
    }
  }

  get active() {
    return this.related.size > 0;
  }

  contains(activity) {
    return this.related.has(activity.id);
  }
}
