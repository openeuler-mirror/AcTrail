export function buildOverviewSections(traceDetail, actionTree) {
  const trace = traceDetail?.trace ?? {};
  const counts = traceDetail?.counts ?? {};
  const semantic = actionTree?.summary ?? {};
  return [
    {
      title: 'Trace',
      items: compactItems([
        ['Name', trace.name],
        ['Profile', trace.profile],
        ['Root PID', trace.root_pid],
        ['State', trace.state],
        ['Health', trace.health],
        ['Created', formatTime(trace.created_at)],
        ['Started', formatTime(trace.started_at)],
        ['Completed', formatTime(trace.completed_at)],
      ]),
    },
    {
      title: 'Events',
      items: compactItems([
        ['Total', counts.events],
        ['Process', counts.process],
        ['File', counts.file],
        ['Network', counts.net],
        ['Resource', counts.resource],
        ['Payload Bytes', counts.retained_payload_bytes],
      ]),
    },
    {
      title: 'Semantic',
      items: compactItems([
        ['Actions', semantic.actions ?? actionTree?.actions?.length],
        ['Links', semantic.links ?? actionTree?.links?.length],
        ['Roots', semantic.roots ?? actionTree?.roots?.length],
      ]),
    },
  ];
}

function formatTime(value) {
  if (!value) {
    return '';
  }
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function compactItems(items) {
  return items
    .filter(([, value]) => value !== undefined && value !== null && value !== '')
    .map(([label, value]) => ({ label, value }));
}
