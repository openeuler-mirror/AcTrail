"""Traverse the same finite child pages used by the action-tree UI."""
import json
import urllib.parse
import urllib.request


class LazyTreeReader:
    def __init__(self, base_url, timeout_seconds, page_limit=100):
        self.base_url = base_url
        self.timeout_seconds = timeout_seconds
        self.page_limit = page_limit

    def pages(self):
        stack = ['agent-process']
        expanded = set()
        while stack:
            parent = stack.pop()
            if parent in expanded:
                continue
            expanded.add(parent)
            offset = 0
            while True:
                encoded = urllib.parse.quote(parent, safe='')
                url = f'{self.base_url}/children/{encoded}?offset={offset}&limit={self.page_limit}'
                with urllib.request.urlopen(url, timeout=self.timeout_seconds) as response:
                    page = json.load(response)
                if len(page.get('actions', [])) > self.page_limit:
                    raise RuntimeError('action-tree child page exceeded requested limit')
                yield parent, page
                stack.extend(row['id'] for row in page.get('child_state', [])
                             if row.get('has_children'))
                if not page.get('has_more'):
                    break
                following = page.get('next_offset')
                if not isinstance(following, int) or following <= offset:
                    raise RuntimeError('action-tree child pagination did not advance')
                offset = following

    def collect(self):
        actions = {}
        links = {}
        for _, page in self.pages():
            actions.update((row['id'], row) for row in page.get('actions', []))
            links.update((json.dumps(row, sort_keys=True), row)
                         for row in page.get('links', []))
        return {'actions': list(actions.values()), 'links': list(links.values())}
