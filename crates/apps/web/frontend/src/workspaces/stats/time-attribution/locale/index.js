/**
 * Copy owned by the time-attribution stats view. Components read it through
 * `useModuleLocale`, so the language stays application-wide.
 */
export default {
  'en-US': {
    header: {
      kicker: 'Wall-clock attribution and tool workloads',
      title: 'Agent / model time attribution',
      note: 'Trace intervals that overlap the selected range are clipped before aggregation.',
      from: 'From',
      to: 'To',
      refresh: 'Refresh',
    },
    summary: {
      totalScope: 'Total attributed scope',
      coverage: '{traces} traces · {paired} paired model calls · {attributed} attributed · {toolIntervals} Agent Tool intervals · {commands} command processes',
    },
    models: {
      title: 'Models',
      note: 'Observable model-side wall time',
      callsIntervals: '{calls} calls · {intervals} intervals',
      empty: 'No model time in range.',
    },
    tools: {
      title: 'Agent Tools',
      note: 'Each real tool invocation; overlapping tools retain their own wall time',
      actionsIntervals: '{actions} actions · {intervals} intervals',
      empty: 'No Agent Tool invocations in range.',
    },
    commands: {
      title: 'Commands',
      note: 'Actual command process trees, counted exclusively',
      countLabel: '{processes} command processes · {intervals} intervals',
      toolOverheadLabel: '{intervals} intervals · Agent Tool self-time',
      viaTool: 'via Agent Tool: {tools}',
      empty: 'No actual commands in range.',
    },
    traces: {
      title: 'Matching traces',
      contribution: 'Longest contiguous interval in this Trace · {duration} total contribution',
      selected: '{label} · {count} traces',
      selectHint: 'Select a category, model, Agent Tool, or command to drill down.',
      loading: 'Loading trace intervals…',
      noMatch: 'No traces match this item and the global filter.',
      idle: 'Aggregates remain query-light until a drill-down item is selected.',
      row: 'Trace {id} · {status}',
      overlapShare: '{percent} overlap-counted workload / clipped Trace',
      clippedShare: '{percent} of clipped Trace',
      loadMore: 'Load more',
    },
    issues: {
      title: 'Collection status',
    },
    source: 'Stats Time Attribution',
  },
  'zh-CN': {
    header: {
      kicker: '墙钟耗时归因与工具工作量',
      title: 'Agent / 模型耗时归因',
      note: '与所选区间重叠的Trace区间会先裁剪，再参与汇总。',
      from: '起始',
      to: '结束',
      refresh: '刷新',
    },
    summary: {
      totalScope: '已归因范围总计',
      coverage: '{traces}条Trace · {paired}次配对模型调用 · {attributed}次已归因 · {toolIntervals}个Agent Tool区间 · {commands}个命令进程',
    },
    models: {
      title: '模型',
      note: '可观测的模型侧墙钟耗时',
      callsIntervals: '{calls}次调用 · {intervals}个区间',
      empty: '该区间内没有模型耗时。',
    },
    tools: {
      title: 'Agent Tool',
      note: '每次真实工具调用；重叠的工具各自保留自身墙钟耗时',
      actionsIntervals: '{actions}次执行 · {intervals}个区间',
      empty: '该区间内没有Agent Tool调用。',
    },
    commands: {
      title: '命令',
      note: '真实命令进程树，按互斥方式计数',
      countLabel: '{processes}个命令进程 · {intervals}个区间',
      toolOverheadLabel: '{intervals}个区间 · Agent Tool自身耗时',
      viaTool: '经由Agent Tool：{tools}',
      empty: '该区间内没有真实命令。',
    },
    traces: {
      title: '匹配的Trace',
      contribution: '该Trace中最长的连续区间 · 贡献合计{duration}',
      selected: '{label} · {count}条Trace',
      selectHint: '选择某个类别、模型、Agent Tool或命令以下钻。',
      loading: '正在加载Trace区间…',
      noMatch: '没有Trace同时匹配该项与全局筛选。',
      idle: '未选择下钻项时，汇总仅做轻量查询。',
      row: 'Trace {id} · {status}',
      overlapShare: '{percent}重叠计入的工作量 / 裁剪后的Trace',
      clippedShare: '占裁剪后Trace的{percent}',
      loadMore: '加载更多',
    },
    issues: {
      title: '采集状态',
    },
    source: '统计耗时归因',
  },
};
