import { compareActivities, packActivityDepths } from './utils.js';

/** A window-local projection of stored command links; no ancestry is inferred. */
export class CommandHierarchy {
  constructor(activities, links = []) {
    this.activities = [...activities].sort(compareActivities);
    this.byId = new Map(this.activities.map((activity) => [activity.id, activity]));
    this.parents = new Map();
    this.ambiguous = new Set();
    for (const activity of this.activities) {
      if (activity.action?.attributes?.['process.parent.identity_state'] === 'conflict') {
        this.ambiguous.add(activity.id);
      }
    }
    this.readParents(links);
  }

  readParents(links) {
    for (const link of links) {
      if (link.valid === false || !this.byId.has(link.child) || this.ambiguous.has(link.child)) {
        continue;
      }
      const parent = this.byId.get(link.parent);
      const child = this.byId.get(link.child);
      const commandChild = child.kind === 'command.invocation';
      const commandParent = link.role === 'command.contains_command_invocation'
        && (!parent || parent.kind === 'command.invocation')
        && (commandChild || child.kind === 'agent.invocation');
      const agentParent = link.role === 'agent.invocation.exec'
        && (!parent || parent.kind === 'agent.invocation') && commandChild;
      if (!commandParent && !agentParent) {
        continue;
      }
      const previous = this.parents.get(link.child);
      if (link.parent === link.child || (previous && previous !== link.parent)) {
        this.ambiguous.add(link.child);
        this.parents.set(link.child, null);
      } else {
        this.parents.set(link.child, link.parent);
      }
    }
  }

  levels() {
    const levels = new Map();
    for (const activity of this.activities) {
      const path = [];
      const positions = new Map();
      let cursor = activity.id;
      while (cursor && !levels.has(cursor)) {
        if (positions.has(cursor)) {
          for (const id of path.slice(positions.get(cursor))) {
            this.ambiguous.add(id);
            levels.set(id, 0);
          }
          break;
        }
        positions.set(cursor, path.length);
        path.push(cursor);
        const parentId = this.parents.get(cursor);
        cursor = this.byId.has(parentId) ? parentId : null;
      }
      for (let index = path.length - 1; index >= 0; index -= 1) {
        const id = path[index];
        if (!levels.has(id)) {
          const parentLevel = levels.get(this.parents.get(id));
          levels.set(id, parentLevel === undefined ? 0 : parentLevel + 1);
        }
      }
    }
    return levels;
  }

  pack() {
    const levels = this.levels();
    const activities = [];
    for (const activity of this.activities) {
      const level = levels.get(activity.id);
      const parentId = this.parents.get(activity.id) ?? null;
      activities.push({
        ...activity,
        commandHierarchy: {
          level,
          parentId,
          parentLabel: this.byId.get(parentId)?.label ?? null,
          relationship: this.ambiguous.has(activity.id) ? 'ambiguous'
            : parentId ? this.byId.has(parentId) ? 'linked-visible' : 'linked-parent-outside-view'
            : 'root',
        },
      });
    }
    return CommandHierarchy.packLevels(activities);
  }

  static packVisible(activities) {
    const visible = new Set(activities.map(activity => activity.id));
    return this.packLevels(activities.map(activity => {
      const hierarchy = activity.commandHierarchy;
      return {
        ...activity,
        commandHierarchy: {
          ...hierarchy,
          relationship: hierarchy.parentId && hierarchy.relationship !== 'ambiguous'
            ? visible.has(hierarchy.parentId) ? 'linked-visible' : 'linked-parent-outside-view'
            : hierarchy.relationship,
        },
      };
    }));
  }

  static packLevels(rows) {
    const byLevel = new Map();
    for (const activity of rows) {
      const level = activity.commandHierarchy.level;
      if (!byLevel.has(level)) byLevel.set(level, []);
      byLevel.get(level).push(activity);
    }
    const activities = [];
    const commandLevels = [];
    let depthCount = 0;
    for (const [level, rows] of [...byLevel].sort(([left], [right]) => left - right)) {
      commandLevels.push({ level, depth: depthCount });
      const packed = packActivityDepths(rows);
      for (const activity of packed.activities) {
        activities.push({ ...activity, depth: depthCount + activity.depth });
      }
      depthCount += packed.depthCount;
    }
    return { activities, depthCount: Math.max(depthCount, 1), commandLevels };
  }

  static detailRows(activity, t) {
    const hierarchy = activity?.density ? null : activity?.commandHierarchy;
    return hierarchy ? {
      command_level: hierarchy.level,
      command_parent: hierarchy.parentLabel ?? hierarchy.parentId
        ?? t(hierarchy.relationship === 'root' ? 'commandHierarchy.rootParent' : 'commandHierarchy.ambiguousParent'),
      ...(hierarchy.parentId ? { command_parent_action: hierarchy.parentId } : {}),
      command_relationship: this.relationshipText(hierarchy, t),
    } : {};
  }

  static relationshipText(hierarchy, t) {
    return hierarchy ? t(`commandHierarchy.${hierarchy.relationship}`) : '';
  }
}
