"""Read expected request content from a real captured SQLite database."""
import json


class CapturedRequest:
    def __init__(self, database, trace_id):
        self.database = database
        self.trace_id = trace_id

    def read(self):
        manifest = self.database.execute(
            'SELECT m.manifest_id,m.skeleton_json,ids.action_id '
            'FROM llm_request_manifests m JOIN semantic_action_ids ids USING(action_key) '
            'WHERE m.trace_id=? ORDER BY m.manifest_id DESC LIMIT 1', (self.trace_id,)).fetchone()
        if manifest is None:
            raise RuntimeError('real agent capture has no persisted request manifest')
        blocks = {}
        for ordinal, raw, size, trace in self.database.execute(
                'SELECT r.ordinal,b.encoded_bytes,b.uncompressed_bytes,b.trace_id '
                'FROM llm_request_block_refs r JOIN llm_request_blocks b USING(block_id) '
                'WHERE r.manifest_id=?', (manifest[0],)):
            assert trace == self.trace_id and len(raw) == size
            blocks[ordinal] = json.loads(raw)
        block_pointers = []

        def hydrate(value, pointer=''):
            if isinstance(value, dict):
                if set(value) == {'$actrail_llm_block'}:
                    block_pointers.append(pointer)
                    return blocks[value['$actrail_llm_block']]
                return {key: hydrate(child, pointer + '/' + key.replace('~', '~0').replace('/', '~1'))
                        for key, child in value.items()}
            if isinstance(value, list):
                return [hydrate(child, pointer + '/' + str(index)) for index, child in enumerate(value)]
            return value

        body = hydrate(json.loads(manifest[1]))
        if not block_pointers:
            raise RuntimeError('real request must include content blocks for bounded-read acceptance')
        return {'action_id': manifest[2], 'body': body, 'block_pointers': block_pointers}
