"""Read actual HTTPS responses and retained process/action evidence after timing."""
from __future__ import annotations

import collections
import json
import re
import subprocess


class SampleEvidence:
    def __init__(self, group, directory, bins, count):
        self.group, self.directory, self.bins, self.count = group, directory, bins, count

    def business(self, clients):
        records = [json.loads(line) for line in (self.directory / 'maas-client/maas.log').read_text().splitlines()
                   if line.startswith('{')]
        requests = [record for record in records if record.get('event') == 'local_maas_request']
        if len(requests) != self.count or any(record['status'] != 200 for record in requests):
            raise RuntimeError('MaaS did not complete the expected real HTTPS request count')
        for client in clients:
            index = client['index']
            client['pid'] = int((self.directory / f'client-{index}.pid').read_text())
            text = (self.directory / f'client-{index}.sse').read_text()
            if '[DONE]' not in text:
                raise RuntimeError('curl did not receive SSE completion')
            content = ''
            for line in text.splitlines():
                if line.startswith('data: ') and line[6:] != '[DONE]':
                    message = json.loads(line[6:])
                    for choice in message.get('choices', []):
                        content += choice.get('delta', {}).get('content') or ''
            if content != f'LIBRARY_HTTPS_RESPONSE_{index}':
                raise RuntimeError('curl response content or sequential request association mismatch')
        if len({client['pid'] for client in clients}) != self.count:
            raise RuntimeError('curl PIDs are not unique within this serial sample')
        return dict(curl_https_requests=self.count, completed_sse=self.count, successful_curl_processes=self.count,
                    outer_https_requests=2, outer_tool_invocations=1)

    def observed(self, collection, previous, clients):
        traces = collection.query('SELECT trace_id,lifecycle_state,health FROM traces WHERE trace_id>?', (previous,))
        if len(traces) != 1 or traces[0][1] not in ('completed', 'exited') or traces[0][2] != 'clean':
            raise RuntimeError(f'trace is not clean and finalized: {traces}')
        trace = traces[0][0]
        output = subprocess.run([str(self.bins / 'actrailviewer'), '--config', str(self.group / 'runtime-P/actraild.conf'),
            '--output-format', 'json', 'actions', '--trace-id', str(trace)],
            capture_output=True, text=True, check=True, timeout=60)
        (self.directory / 'actions.json').write_text(output.stdout)
        graph = json.loads(output.stdout)
        by_id = {action['action_id']: action for action in graph['actions']}
        by_process = collections.defaultdict(collections.Counter)
        actions_by_process = collections.defaultdict(list)
        for action in graph['actions']:
            by_process[action['process']['process_id']][action['kind']] += 1
            actions_by_process[action['process']['process_id']].append(action)
        for link in graph['links']:
            if link['role'] in ('llm.call.request', 'llm.call.response', 'command.contains_llm_call'):
                if not link['valid'] or by_id[link['parent_action_id']]['process'] != by_id[link['child_action_id']]['process']:
                    raise RuntimeError('present LLM or command link crosses process identity')
        root = collection.query('SELECT root_process_id FROM traces WHERE trace_id=?', (trace,))[0][0]
        if any(by_process[root][kind] != 2 for kind in ('llm.request', 'llm.response', 'llm.call')):
            raise RuntimeError('outer real agent lost LLM coverage')
        if any(not self.complete_action(action) for action in actions_by_process[root]
               if action['kind'] in ('llm.request', 'llm.response')):
            raise RuntimeError('outer real agent lacks complete protocol evidence')
        counts = collections.Counter()
        rows = []
        for client in clients:
            identities = collection.query(
                'SELECT p.process_id,p.host_start_ticks,p.host_start_boottime_ns FROM processes p '
                'JOIN memberships m ON m.process_id=p.process_id WHERE m.trace_id=? AND p.host_pid=?',
                (trace, client['pid']))
            if len(identities) != 1:
                raise RuntimeError('actual curl PID lacks unique process membership in this trace')
            process_id, ticks, boot = identities[0]
            execs = [action for action in actions_by_process[process_id]
                     if action['kind'] == 'process.exec' and action['status'] == 'success'
                     and action['attributes'].get('exec.result') == 'success'
                     and action['attributes'].get('exec.path') == client['command'][0]]
            if len(execs) != 1:
                raise RuntimeError('curl PID membership lacks exactly one successful exec of the actual client')
            own = by_process[process_id]
            row = dict(index=client['index'], pid=client['pid'], process_id=process_id,
                host_start_ticks=ticks, host_start_boottime_ns=boot,
                requests=own['llm.request'], responses=own['llm.response'], calls=own['llm.call'])
            row['exec_action_id'] = execs[0]['action_id']
            if any(row[key] > 1 for key in ('requests', 'responses', 'calls')):
                raise RuntimeError('duplicate curl LLM action')
            protocol_actions = [action for action in actions_by_process[process_id]
                                if action['kind'] in ('llm.request', 'llm.response')]
            row['protocol_actions'] = [dict(kind=action['kind'], status=action['status'],
                completeness=action['completeness'], complete=self.complete_action(action)) for action in protocol_actions]
            roles = collections.Counter(link['role'] for link in graph['links']
                if link['valid'] and link['role'] in ('llm.call.request', 'llm.call.response', 'command.contains_llm_call')
                and by_id[link['child_action_id']]['process']['process_id'] == process_id)
            row['relationship_counts'] = dict(roles)
            row['complete_pair'] = (all(row[key] == 1 for key in ('requests', 'responses', 'calls'))
                and all(self.complete_action(action) for action in protocol_actions)
                and all(roles[role] == 1 for role in ('llm.call.request', 'llm.call.response', 'command.contains_llm_call')))
            counts.update({key: row[key] for key in ('requests', 'responses', 'calls')})
            counts['complete_pairs'] += row['complete_pair']
            rows.append(row)
        missing = {key: self.count - counts[key] for key in ('requests', 'responses', 'calls', 'complete_pairs')}
        return dict(trace_id=trace, lifecycle_state=traces[0][1], health=traces[0][2],
            actual_curl_requests=self.count, observed=dict(counts), missing=missing, clients=rows,
            coverage_equal_to_business=missing['complete_pairs'] == 0)

    @staticmethod
    def complete_action(action):
        return (action['status'] == 'success' and action['completeness'] == 'complete'
                and bool(action['start_time_unix_nanos']) and bool(action['end_time_unix_nanos'])
                and (action['kind'] != 'llm.response' or action['attributes'].get('llm.response.done') == 'true'))

    @staticmethod
    def discovery(log, libraries):
        rows = []
        for library in libraries:
            lines = [line for line in log.splitlines() if re.search(rf'\binode={library["inode"]}\b', line)]
            rows.append(dict(inode=library['inode'], lines=lines,
                analysis_started=sum('analysis started' in line for line in lines),
                cache_hits=sum('cache hit' in line for line in lines),
                coalesced=sum('coalesced' in line for line in lines),
                attachment_ready=sum('attachment ready' in line for line in lines)))
        return dict(libraries=rows, note='attachment ready includes reuse; it is not a count of newly installed probes')
