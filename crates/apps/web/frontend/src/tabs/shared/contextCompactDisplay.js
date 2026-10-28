// Look up the implementation already validated by the server's display projection.
export class ContextCompactDisplay {
  static implementation(compact, lookup) {
    const id = compact?.display?.context_compaction?.implementation_llm_call_action_id;
    return id ? lookup(id) ?? null : null;
  }
}
