/**
 * Copy owned by the shared components. Components resolve it through
 * `useModuleLocale`, which keeps the application language and falls back to
 * the shared dictionary.
 */
export default {
  'en-US': {
    llmCall: {
      request: 'Request',
      response: 'Response',
      loading: 'Loading linked message…',
      unlinked: 'No linked message was recorded.',
      retry: 'Retry',
      noContent: 'No message text is available in this action.',
      technicalDetails: 'Technical details',
      started: 'Started',
      ended: 'Ended',
      status: 'Status',
      completeness: 'Completeness',
      invalidAction: 'The linked action does not match the requested message.',
      contentMissing: 'The request body is unavailable.',
    },
    actionNode: {
      jumpControls: 'Jump between sibling LLM calls',
      previousSibling: 'Previous sibling LLM call',
      previous: 'Previous LLM call',
      nextSibling: 'Next sibling LLM call',
      next: 'Next LLM call',
      collapseChildren: 'Collapse action children',
      expandChildren: 'Expand action children',
      collapse: 'Collapse',
      expand: 'Expand',
    },
    dataTable: {
      empty: 'No rows',
      loadAll: 'Load all',
      loadMore: 'Load {count} more ({hidden} hidden)',
    },
    detail: {
      noSelection: 'No selection',
      pathSet: 'Path Set',
      mcpPayloadMissing: 'MCP payload has no canonical content link or retained raw evidence',
      requestInsightsTooLarge: 'Request insights exceed the {kib} KiB view limit. Open Canonical request body to browse the content.',
    },
    fullscreen: {
      exit: 'Exit fullscreen: {label}',
      enter: 'View fullscreen: {label}',
    },
    insight: {
      showing: 'Showing {shown} of {total}',
    },
    llmInsight: {
      loading: 'Loading request insights',
      load: 'Load request insights',
      show: 'Show request insights',
      hide: 'Hide request insights',
    },
    mcpInsight: {
      loading: 'Loading MCP payload',
    },
    progressiveDisclosure: {
      detailLevel: 'Detail level',
      allData: 'All data',
      focused: 'Focused',
      showAll: 'Show {level}',
    },
    canonicalBody: {
      title: 'Canonical request body',
      loadContent: 'Load content',
      unknownSize: 'unknown size',
    },
    timelineOverview: {
      label: 'Trace overview',
      hint: 'W/S zoom · A/D pan',
      dragHint: 'Drag the window to pan; drag either edge to resize; scroll to zoom',
    },
  },
  'zh-CN': {
    llmCall: {
      request: 'Request · 请求',
      response: 'Response · 响应',
      loading: '正在加载关联消息…',
      unlinked: '未记录关联消息。',
      retry: '重试',
      noContent: '此动作中没有可展示的消息正文。',
      technicalDetails: '技术详情',
      started: '开始时间',
      ended: '结束时间',
      status: '状态',
      completeness: '完整性',
      invalidAction: '关联动作与请求的消息不匹配。',
      contentMissing: '请求正文不可用。',
    },
    actionNode: {
      jumpControls: '在同级LLM调用之间跳转',
      previousSibling: '上一个同级LLM调用',
      previous: '上一个LLM调用',
      nextSibling: '下一个同级LLM调用',
      next: '下一个LLM调用',
      collapseChildren: '折叠该动作的子节点',
      expandChildren: '展开该动作的子节点',
      collapse: '折叠',
      expand: '展开',
    },
    dataTable: {
      empty: '暂无数据',
      loadAll: '加载全部',
      loadMore: '再加载{count}行（另有{hidden}行未显示）',
    },
    detail: {
      noSelection: '未选择',
      pathSet: '路径集合',
      mcpPayloadMissing: '该MCP载荷没有规范化内容链接，也没有保留的原始证据',
      requestInsightsTooLarge: '请求洞察超过{kib} KiB的查看上限。请打开“规范化请求体”浏览内容。',
    },
    fullscreen: {
      exit: '退出全屏：{label}',
      enter: '全屏查看：{label}',
    },
    insight: {
      showing: '显示{shown} / {total}',
    },
    llmInsight: {
      loading: '正在加载请求洞察',
      load: '加载请求洞察',
      show: '显示请求洞察',
      hide: '隐藏请求洞察',
    },
    mcpInsight: {
      loading: '正在加载MCP载荷',
    },
    progressiveDisclosure: {
      detailLevel: '详情层级',
      allData: '全部数据',
      focused: '聚焦',
      showAll: '显示{level}',
    },
    canonicalBody: {
      title: '规范化请求体',
      loadContent: '加载内容',
      unknownSize: '大小未知',
    },
    timelineOverview: {
      label: 'Trace总览',
      hint: 'W/S缩放 · A/D平移',
      dragHint: '拖动窗口可平移；拖动两端可调整范围；滚轮可缩放',
    },
  },
};
