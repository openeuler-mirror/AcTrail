/** Copy owned by the LLM trajectory tab. */
export default {
  'en-US': {
    title: 'LLM Trajectory',
    summaryNote: 'Recorded request histories and persisted lineage within this trace.',
    legendStrictPrefix: 'Strict prefix',
    partialGraph: 'This graph is partial because some trajectory data was unavailable.',
    empty: 'No LLM trajectory data for this trace.',
    graphAria: 'LLM trajectory graph',
    stats: {
      appendEdges: 'Append edges',
      forkEdges: 'Fork edges',
      duplicateRoots: 'Duplicate roots',
      stronglyLinked: 'Strongly linked',
      duplicateRatio: 'Duplicate ratio',
      startReason: 'Start reason',
    },
  },
  'zh-CN': {
    title: 'LLM轨迹',
    summaryNote: '该 Trace 内已记录的请求历史与持久化关联。',
    legendStrictPrefix: '严格前缀',
    partialGraph: '部分轨迹数据不可用，因此该图为部分结果。',
    empty: '该Trace没有LLM轨迹数据。',
    graphAria: 'LLM轨迹图',
    stats: {
      appendEdges: '追加边',
      forkEdges: '分叉边',
      duplicateRoots: '重复根节点',
      stronglyLinked: '强关联比例',
      duplicateRatio: '重复节点比例',
      startReason: '起始原因',
    },
  },
};
