// Read Markdown layout fragments and render their connected module trees locally.
"use strict";

class LayoutNode {
  constructor(name, path, directory, label = { status: "", description: "" }) {
    this.name = name;
    this.path = path;
    this.directory = directory;
    this.label = label;
    this.children = new Map();
    this.subtree = null;
    this.moduleRoot = false;
  }

  get module() {
    return this.moduleRoot || this.subtree !== null;
  }

  get expanded() {
    return this.subtree || this;
  }
}

class LayoutTree {
  constructor(documents) {
    this.root = new LayoutNode("./", [], true);
    const fragments = documents.flatMap(({ text }) => this.readRoots(text));
    if (!fragments.length) throw new Error("所选目录中没有可读取的代码布局树。");
    this.attachModules(fragments);
  }

  readRoots(markdown) {
    const roots = [];
    const stack = [];
    let inFence = false;
    let inTree = false;
    for (const line of markdown.split(/\r?\n/)) {
      const stripped = line.trim();
      if (stripped.startsWith("```")) {
        inTree = !inFence && ["```", "```text", "```plaintext"].includes(stripped);
        inFence = !inFence;
        stack.length = 0;
        continue;
      }
      if (!inTree || !stripped) continue;
      const branch = line.match(/^([ │]*)(?:├──|└──)\s*(.*)$/);
      const depth = branch ? Math.floor(branch[1].length / 4) + 1 : 0;
      const body = branch ? branch[2] : stripped;
      while (stack.length && stack.at(-1).depth >= depth) stack.pop();
      let parent = stack.at(-1)?.node;
      const delimiter = /\s+(?:—|#)\s*/.exec(body);
      let name = (delimiter ? body.slice(0, delimiter.index) : body).trim();
      const description = delimiter ? body.slice(delimiter.index + delimiter[0].length) : "";
      const marker = name.match(/^\[(.)\]\s*/);
      const status = marker ? marker[1] : "";
      if (marker) name = name.slice(marker[0].length);
      const directory = name.endsWith("/");
      // Slash-separated directory paths expand; signatures below files stay intact.
      const segments = !parent || parent.directory
        ? name.split("/").filter(segment => segment && segment !== ".")
        : [name];
      const path = [...(parent?.path || []), ...segments];
      const label = { status, description };
      name = segments.length ? segments.at(-1) + (directory ? "/" : "") : "./";
      const node = new LayoutNode(name, path, directory, label);
      if (!parent) {
        node.moduleRoot = true;
        roots.push(node);
      } else {
        for (let index = parent.path.length; index < path.length - 1; index++) {
          const segment = path[index];
          if (!parent.children.has(segment)) {
            parent.children.set(segment, new LayoutNode(segment + "/", path.slice(0, index + 1), true));
          }
          parent = parent.children.get(segment);
        }
        parent.children.set(path.at(-1), node);
      }
      stack.push({ depth, node });
    }
    return roots;
  }

  attachModules(fragments) {
    const nodes = new Map([[JSON.stringify([]), this.root]]);
    // The parent summary points to its source module; expanded content stays owned by that module.
    fragments.sort((left, right) => left.path.length - right.path.length);
    for (const fragment of fragments) {
      const key = JSON.stringify(fragment.path);
      if (!fragment.path.length) {
        this.root = fragment;
      } else if (nodes.has(key)) {
        nodes.get(key).subtree = fragment;
      } else {
        let parent = this.root;
        for (let length = 1; length < fragment.path.length; length++) {
          const path = fragment.path.slice(0, length);
          const parentKey = JSON.stringify(path);
          if (!nodes.has(parentKey)) {
            const node = new LayoutNode(path.at(-1) + "/", path, true);
            parent.children.set(path.at(-1), node);
            nodes.set(parentKey, node);
          }
          parent = nodes.get(parentKey);
        }
        parent.children.set(fragment.path.at(-1), fragment);
      }
      const pending = [fragment];
      while (pending.length) {
        const node = pending.pop();
        nodes.set(JSON.stringify(node.path), node);
        pending.push(...node.children.values());
      }
    }
  }

  get roots() {
    return this.root.moduleRoot ? [this.root] : [...this.root.children.values()];
  }
}

class LayoutViewer {
  static statuses = new Map([
    [" ", ["pending", "○", "待实现"]],
    ["/", ["changing", "↻", "修改中"]],
    ["x", ["done", "✓", "已完成"]],
    ["-", ["removing", "−", "待删除或迁移"]],
  ]);

  constructor() {
    this.tree = document.getElementById("tree");
    this.message = document.getElementById("message");
    this.error = document.getElementById("error");
    this.directory = document.getElementById("directory");
    this.search = document.getElementById("search");
    this.expand = document.getElementById("expand-all");
    this.collapse = document.getElementById("collapse-all");
    this.rows = [];
    this.searching = false;
    this.loadVersion = 0;
    this.directory.addEventListener("change", () => this.load(this.directory.files));
    this.search.addEventListener("input", () => this.filter());
    this.expand.addEventListener("click", () => this.setExpanded(true));
    this.collapse.addEventListener("click", () => this.setExpanded(false));
  }

  async load(selection) {
    const version = ++this.loadVersion;
    const files = [...selection].filter(file => file.name === "code-layout.md")
      .sort((left, right) => left.webkitRelativePath.localeCompare(right.webkitRelativePath));
    this.error.hidden = true;
    this.message.textContent = "正在读取布局文件…";
    try {
      if (!files.length) throw new Error("所选目录中没有 code-layout.md，请选择 docs/architecture 目录。");
      const documents = await Promise.all(files.map(async file => ({ text: await file.text() })));
      if (version !== this.loadVersion) return;
      const layout = new LayoutTree(documents);
      this.rows = [];
      const content = document.createDocumentFragment();
      for (const root of layout.roots) content.append(this.render(root, 0, true));
      this.tree.replaceChildren(content);
      this.search.value = "";
      this.searching = false;
      this.search.disabled = this.expand.disabled = this.collapse.disabled = false;
      this.message.textContent = `已读取 ${files.length} 个布局文件，共 ${this.rows.length} 个节点。`;
    } catch (error) {
      if (version !== this.loadVersion) return;
      this.error.textContent = error.message;
      this.error.hidden = false;
      this.message.textContent = "读取失败，请重新选择布局目录。";
    } finally {
      if (version === this.loadVersion) this.directory.value = "";
    }
  }

  label(node, label) {
    const content = document.createElement("span");
    const status = document.createElement("span");
    status.className = "status";
    const definition = LayoutViewer.statuses.get(label.status);
    if (definition) {
      status.classList.add(definition[0]);
      status.textContent = definition[1];
      status.title = definition[2];
      status.setAttribute("aria-label", definition[2]);
    }
    const name = document.createElement("span");
    name.className = "name";
    name.textContent = node.name;
    const description = document.createElement("span");
    description.className = "description";
    description.textContent = label.description;
    content.append(status, name, description);
    return content;
  }

  render(node, moduleDepth, root = false) {
    const children = [...node.expanded.children.values()];
    const branch = children.length > 0 || node.module;
    const element = document.createElement(branch ? "details" : "div");
    element.className = "node";
    if (node.module) {
      element.classList.add("module");
      element.style.setProperty("--hue", (250 + moduleDepth * 60) % 360);
      moduleDepth++;
    }
    const row = {
      element, branch, children: [],
      text: `${node.path.join("/")} ${node.label.description} ${node.expanded.label.description}`.toLowerCase(),
    };
    this.rows.push(row);
    const label = this.label(node, node.label);
    if (!branch) {
      element.append(label);
      return element;
    }
    element.open = root;
    const summary = document.createElement("summary");
    label.className = "collapsed-label";
    const expanded = this.label(node, node.expanded.label);
    expanded.className = "expanded-label";
    summary.append(label, expanded);
    const container = document.createElement("div");
    container.className = "children";
    for (const child of children) {
      const index = this.rows.length;
      container.append(this.render(child, moduleDepth));
      row.children.push(this.rows[index]);
    }
    element.append(summary, container);
    return element;
  }

  setExpanded(open) {
    for (const row of this.rows) {
      if (row.branch) row.element.open = open;
    }
  }

  filter() {
    const query = this.search.value.trim().toLowerCase();
    if (query && !this.searching) {
      for (const row of this.rows) row.previousOpen = row.element.open;
    }
    // Work from leaves upward so matched descendants keep their ancestors visible.
    for (let index = this.rows.length - 1; index >= 0; index--) {
      const row = this.rows[index];
      const matches = !query || row.text.includes(query) || row.children.some(child => !child.element.hidden);
      row.element.hidden = !matches;
      if (row.branch) {
        if (query) row.element.open = matches;
        else if (this.searching) row.element.open = row.previousOpen;
      }
    }
    this.searching = Boolean(query);
  }
}

new LayoutViewer();
