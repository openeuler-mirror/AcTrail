# Web 前端代码布局

本文记录 `actrailweb` 前端源码职责和依赖方向。文档中的源码位置统一相对于 `crates/apps/web/frontend/src/`；PUML 中的路径是普通文本，不要求生成可点击链接。

## 目录结构

```text
crates/apps/web/
├── build.rs                         # Vite 构建与嵌入式 asset table
├── src/render.rs                    # 运行时静态资源查找
└── frontend/
    ├── package.json                 # Vue、Vite 与 lucide 版本
    ├── vite.config.js               # 构建产物命名
    └── src/
        ├── main.js                  # Vue 挂载入口
        ├── App.vue                  # 应用壳、根状态和跨 Workspace 编排
        ├── api.js                   # 浏览器 HTTP 请求边界
        ├── styles.css               # 应用壳几何、壳层组件样式与核心断点
        ├── workspaces/              # Statistics、Config、Plugins、Traces
        ├── tabs/                    # Trace leaf views、registry 与共享表格投影
        ├── components/              # 表格、详情和 insight 控件
        ├── locale/                  # 界面语言资源
        └── theme/                   # 主题 manifest、角色 token 与共享契约
```

## 主题组织

- `theme/contract.css` 声明主题无关的几何、排版、动效 token，并派生 `--trace-*` 组件 token。
- `theme/index.js` 暴露 `system`、`light`、`white`、`dark` 四个模式。`system` 跟随平台配色并在其变化时重新解析为 `white` 或 `dark`，其余三个模式固定色板；解析结果写入根元素的 `data-palette`，模式写入 `data-theme-mode`。
- `theme/light/`、`theme/white/` 与 `theme/dark/` 各提供一份角色色板。暖纸色板随应用 bundle 加载，另外两份由加载器按需注入，因此首屏不会因为等待样式而闪烁。
- 角色色板定义在 `:root` 与 `:root[data-palette='white'|'dark']`，body 与浮层同样继承当前色板。
- 玻璃与动效令牌由契约提供，并在窄视口、`prefers-reduced-motion` 或 `prefers-reduced-transparency` 下降级为实色与零时长。
- 角色 token 命名统一为 `--ui-*`；色板只覆盖颜色角色，几何与派生 token 由契约提供。

## 依赖方向

- `main.js` 只负责加载全局样式、创建应用并挂载 `App.vue`。
- `App.vue` 可以依赖 Workspace、全局 API 和应用级控件；Workspace 不反向依赖 `App.vue`。
- Workspace 负责数据加载、请求失效、跨视图用例和 leaf view 状态。
- `tabs/registry.js` 提供 Trace leaf view 的稳定 ID、标签、组件映射和分组 descriptor，不接管选择状态或数据加载。
- 领域 view 可以组合共享控件；共享控件不得导入 Workspace 或 leaf registry。
- `api.js` 不依赖 Vue 页面和组件。

## 控件路径表达

架构图只标注对布局、状态或依赖边界有意义的控件。节点使用下列格式：

```text
控件名称
职责
相对于 crates/apps/web/frontend/src/ 的路径
```

叶子按钮、图标和仅用于局部排版的元素不进入全局组件图。复杂领域控件可以拥有单独文档；其入口组件记录精确路径，内部实现只记录目录边界，避免让架构图退化成文件清单。

## 拆分规则

- 只有稳定的交互语义或被两个明确调用方复用时，才抽取共享控件。
- 不抽取同时接管导航、业务状态、数据加载和动态页面渲染的万能容器。
- Workspace 保持自身状态高内聚；共享控件通过 props、events、`v-model` 和 slots 组合。
- 新目录应声明唯一公共入口，其余实现保持目录私有；导出范围遵循最小化原则。
- CSS 变量和主题 token 是稳定视觉值的代码真相，Markdown 不复制完整数值表。
