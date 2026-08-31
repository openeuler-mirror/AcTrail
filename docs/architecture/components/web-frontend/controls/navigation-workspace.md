# Navigator 与 Workspace

本文定义 `actrailweb` 导航控件与 Workspace 的职责边界，并记录 Trace Workspace 当前使用的两级导航信息架构。

## 当前状态归属

- `App.vue` 持有顶层 Workspace ID，并选择 Active Workspace。
- `GlobalTabs.vue` 是受控的顶层导航，只显示选项并发出选择。
- `TraceWorkspace.vue` 持有所选 Trace、活动 leaf view、每组最近选择、按需数据、加载错误和请求竞态令牌。
- `NavigationStrip.vue` 是受控导航原语，分别渲染 Trace 一级和二级导航，不加载数据，也不解析 leaf component。
- `tabs/registry.js` 保存稳定 leaf ID、component 映射和四组 descriptor。

导航状态、数据生命周期和页面渲染目前集中在正确的最近所属组件中，不应把整套 Navigator + Workspace 抽成通用容器。

## 可复用控件边界

`NavigationStrip` 只负责：

- 渲染 `id`、`label`、可选 icon、badge 和 disabled 状态；
- 标记 active item，并发出 `update:modelValue` 与 `select`；
- 提供 page navigation 或 tablist 对应的 ARIA 语义；
- 实现方向键、焦点和横向溢出策略；
- 通过 item slot 扩展单项内容。

它不得负责：

- Trace 选择、Metrics strip 或 Active Workspace 布局；
- leaf registry 解析和动态 component 生命周期；
- API 懒加载、缓存、错误、请求取消或竞态保护；
- group 默认项、最近访问记忆和跨页面 deep link；
- Waterfall 与 Time Attribution 等领域跳转。

这些职责继续由 `TraceWorkspace` 持有。通用控件目前只服务 Trace 的一级和二级导航；GlobalTabs、Statistics rail 和 LLM 内部导航尚未迁移。

## Trace 信息架构

一级和二级导航均不得超过 6 项：

| 一级视图 | 二级视图 | 默认项 |
|---|---|---|
| Overview | 无 | Overview |
| Execution | Actions、Commands、Processes、Process Tree、Waterfall、Time Attribution | Actions |
| Activity | Timeline、Events、Files、Network、Payloads、Resources | Timeline |
| Health | Alerts、Diagnostics | Alerts |

`activeViewId` 是唯一 canonical selection。`activeGroupId` 从 registry descriptor 派生，不能与 leaf ID 分别维护为两个可能冲突的状态。Workspace 可以使用 `lastViewByGroup` 记住每组最近访问项；首次进入组时选择表中的默认项。

现有 leaf ID 保持稳定，跨 Workspace 打开 Trace 时仍传递 leaf ID。按 leaf view 懒加载数据，进入一级分组不能一次请求组内全部数据；单个下游请求失败只影响对应 leaf view。

## 响应式和无障碍

- 宽屏同时显示一级和当前组的二级导航。
- 窄屏允许二级导航变为选择控件，但不能改变 canonical leaf ID。
- page navigation 使用 `nav` 与 `aria-current`；页面内部切换使用 `tablist`、`tab`、`aria-selected` 和对应 panel 关系。
- 键盘焦点与 active selection 分离；方向键移动焦点，Enter 或 Space 确认选择。
- 动态视图切换后，加载和错误状态由 Workspace 局部呈现，不能导致整个应用壳崩溃。

## 实现入口

| 职责 | 相对源码路径 |
|---|---|
| 应用根与顶层状态 | `App.vue` |
| 顶层导航 | `workspaces/GlobalTabs.vue` |
| Trace 状态与加载 | `workspaces/TraceWorkspace.vue` |
| 受控导航原语 | `components/navigation/NavigationStrip.vue` |
| leaf view 与 group registry | `tabs/registry.js` |
