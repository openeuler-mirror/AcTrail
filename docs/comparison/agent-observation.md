<table>
  <thead>
    <tr>
      <th>类别</th>
      <th>对象 / 特征</th>
      <th><a href="../concepts/collection-observation-checklist.md">AcTrail</a></th>
      <th><a href="https://code.claude.com/docs/en/hooks">日志/hooks</a></th>
      <th><a href="https://opentelemetry.io/docs/concepts/instrumentation/">SDK 手工埋点</a></th>
      <th><a href="https://agentgateway.dev/docs/standalone/latest/documentation/observability/">LLM/MCP 网关</a></th>
      <th><a href="https://tetragon.io/docs/overview/">Tetragon(系统观测)</a></th>
      <th><a href="https://langfuse.com/docs/observability/overview">Langfuse</a></th>
    </tr>
  </thead>
  <tbody align="center">
    <tr><th>观测手段</th><td>数据来源</td><td>系统探针</td><td>Agent 事件</td><td>应用埋点</td><td>网关流量</td><td>系统探针</td><td>接入数据</td></tr>
    <tr><th rowspan="2">观测限制</th><td>无需修改 Agent 业务代码</td><td>✅</td><td>✅</td><td>❌</td><td>✅</td><td>✅</td><td>✅ 网关接入</td></tr>
    <tr><td>无需改变模型/工具请求路由</td><td>✅</td><td>✅</td><td>✅</td><td>❌</td><td>✅</td><td>✅ SDK 接入</td></tr>
    <tr><th rowspan="6">观测能力<br>实体</th><td>运行的进程</td><td>✅</td><td>❌</td><td>❌</td><td>❌</td><td>✅</td><td>❌</td></tr>
    <tr><td>访问的文件</td><td>✅</td><td>❌</td><td>❌</td><td>❌</td><td>✅</td><td>❌</td></tr>
    <tr><td>建立的网络连接</td><td>✅</td><td>❌</td><td>❌</td><td>❌</td><td>✅</td><td>❌</td></tr>
    <tr><td>模型请求、响应</td><td>✅</td><td>✅</td><td>✅</td><td>✅</td><td>❌</td><td>✅</td></tr>
    <tr><td>工具调用、结果</td><td>✅</td><td>✅</td><td>✅</td><td>✅</td><td>❌</td><td>✅</td></tr>
    <tr><td>自定义业务步骤</td><td>❌</td><td>✅</td><td>✅</td><td>❌</td><td>❌</td><td>✅</td></tr>
    <tr><th rowspan="7">观测能力<br>关系</th><td>进程父子关系</td><td>✅</td><td>❌</td><td>❌</td><td>❌</td><td>✅</td><td>❌</td></tr>
    <tr><td>进程的文件读写</td><td>✅</td><td>❌</td><td>❌</td><td>❌</td><td>✅</td><td>❌</td></tr>
    <tr><td>进程的网络连接</td><td>✅</td><td>❌</td><td>❌</td><td>❌</td><td>✅</td><td>❌</td></tr>
    <tr><td>MCP stdio 通信双方关联</td><td>✅</td><td>❌</td><td>❌</td><td>❌</td><td>❌</td><td>❌</td></tr>
    <tr><td>模型请求与响应配对</td><td>✅</td><td>✅</td><td>✅</td><td>✅</td><td>❌</td><td>✅</td></tr>
    <tr><td>模型发起的工具调用</td><td>✅</td><td>✅</td><td>✅</td><td>✅</td><td>❌</td><td>✅</td></tr>
    <tr><td>工具调用与结果配对</td><td>✅</td><td>✅</td><td>✅</td><td>✅</td><td>❌</td><td>✅</td></tr>
  </tbody>
  <tfoot>
    <tr><td colspan="8">✅ 支持 · ❌ 无原生能力 ｜ 范围：已启用采集、支持的协议</td></tr>
  </tfoot>
</table>
