// A real Node HTTP/2 client: one TLS session, concurrently active request streams.
import http2 from 'node:http2';
import fs from 'node:fs';

const manifest = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
const session = http2.connect(`https://127.0.0.1:${manifest.port}`, {
  ca: fs.readFileSync(manifest.cert), servername: 'localhost',
});
const timer = setTimeout(() => { session.destroy(); process.exitCode = 1; }, 20000);
try {
  await new Promise((resolve, reject) => {
    session.once('connect', resolve);
    session.once('error', reject);
  });
  if (session.alpnProtocol !== 'h2') throw new Error('HTTP/2 ALPN required');
  const results = await Promise.all(manifest.cases.map(item => new Promise((resolve, reject) => {
    const request = session.request({ ':method': 'POST', ':path': '/v1/chat/completions',
      'content-type': 'application/json' });
    const received = [];
    let error;
    request.on('data', chunk => received.push(chunk));
    request.on('error', value => { error = value; });
    request.on('close', () => {
      const body = Buffer.concat(received).toString();
      if (body !== item.response || request.rstCode !== (item.reset ? 8 : 0)) {
        reject(new Error(`${item.name}: bytes/reset mismatch (${request.rstCode}): ${error}`));
        return;
      }
      resolve({ name: item.name, stream_id: request.id, reset: request.rstCode, bytes: Buffer.byteLength(body) });
    });
    request.end(JSON.stringify(item.request));
  })));
  fs.writeFileSync(manifest.result, JSON.stringify({ alpn: session.alpnProtocol, streams: results }, null, 2));
} finally {
  clearTimeout(timer);
  session.close();
}
