import { row, windowedTableView } from '../../tableModel';
const COLUMNS = Object.freeze([
  {key:'time',label:'Time'}, {key:'process',label:'Process identity'},
  {key:'kind',label:'Sample'}, {key:'cpu',label:'CPU %'},
  {key:'rss',label:'RSS KiB'}, {key:'memory',label:'Cgroup memory bytes'},
  {key:'source',label:'Source'}, {key:'coverage',label:'Coverage'},
]);
export function project({traceDetail,query='',rowLimit=0}) {
  return windowedTableView(COLUMNS,traceDetail?.resources,resourceRow,'No resource samples',{query,rowLimit});
}
function resourceRow(sample,index) {
  const metrics=sample.payload;
  const at=sample.observed_at;
  const fields = {
    time:new Date(at.secs_since_epoch*1000+at.nanos_since_epoch/1e6).toISOString(),
    process:`process-${sample.process}`,kind:metrics.sample_kind,
    cpu:metrics.cpu_percent_millis==null?'unknown':(metrics.cpu_percent_millis/1000).toFixed(3),
    rss:metrics.rss_kb??'unknown',memory:metrics.memory_current_bytes??'unknown',
    source:metrics.accounting_method,coverage:metrics.accounting_coverage,
  };
  return row(`${sample.trace_id}-${sample.process}-${at.secs_since_epoch}-${at.nanos_since_epoch}-${index}`, fields, {
    title: `Resource sample · trace-${sample.trace_id} · process-${sample.process}`,
    kind: 'resource',
    rows: fields,
    attributes: metrics.metadata ?? {},
    raw: sample,
  });
}
