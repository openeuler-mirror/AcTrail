<table>
  <thead>
    <tr>
      <th>类别</th>
      <th>控制项 / 特征</th>
      <th><a href="../../crates/access_control/README.md">AcTrail</a></th>
      <th><a href="https://docs.docker.com/engine/network/firewall-iptables/">Docker＋iptables</a></th>
      <th><a href="https://docs.kernel.org/userspace-api/landlock.html">Landlock</a></th>
      <th><a href="https://nsjail.dev/">nsjail</a></th>
      <th><a href="https://github.com/containers/bubblewrap">bwrap</a></th>
      <th><a href="https://docs.kernel.org/bpf/prog_lsm.html">BPF LSM 策略</a></th>
    </tr>
  </thead>
  <tbody align="center">
    <tr><th rowspan="5">执行约束</th><td>文件读写权限</td><td>✅</td><td>✅</td><td>✅</td><td>✅</td><td>✅</td><td>✅</td></tr>
    <tr><td>允许执行的程序</td><td>✅</td><td>❌</td><td>✅</td><td>❌</td><td>❌</td><td>✅</td></tr>
    <tr><td>允许连接的 TCP 端口</td><td>✅</td><td>✅</td><td>✅</td><td>❌</td><td>❌</td><td>✅</td></tr>
    <tr><td>允许绑定的 TCP 端口</td><td>✅</td><td>❌</td><td>✅</td><td>❌</td><td>❌</td><td>✅</td></tr>
    <tr><td>允许访问的 IP/CIDR</td><td>✅</td><td>✅</td><td>❌</td><td>❌</td><td>❌</td><td>✅</td></tr>
    <tr><th>策略更新</th><td>运行中更新、撤销规则</td><td>✅</td><td>✅ 网络规则</td><td>❌</td><td>❌</td><td>❌</td><td>✅</td></tr>
    <tr><th rowspan="2">决策扩展</th><td>治理插件及授权管理</td><td>✅</td><td>❌</td><td>❌</td><td>❌</td><td>❌</td><td>❌</td></tr>
    <tr><td>系统调用等待外部授权决定</td><td><a href="../../crates/access_control/kernel/README.md">✅ 扩展模块</a></td><td>❌</td><td>❌</td><td>❌</td><td>❌</td><td>❌</td></tr>
    <tr><th>环境隔离</th><td>文件系统、进程、网络隔离</td><td>❌</td><td>✅</td><td>❌</td><td>✅</td><td>✅</td><td>❌</td></tr>
    <tr><th>资源限制</th><td>CPU、内存、进程数上限</td><td>❌</td><td>✅</td><td>❌</td><td>✅</td><td>❌</td><td>❌</td></tr>
  </tbody>
  <tfoot>
    <tr><td colspan="8">✅ 支持 · ❌ 无原生能力 ｜ 范围：所列组件、已启用规则、支持的内核</td></tr>
  </tfoot>
</table>
