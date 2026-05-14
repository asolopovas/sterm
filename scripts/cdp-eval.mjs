const expr = process.argv.slice(2).join(' ');
const targets = await fetch('http://localhost:9223/json').then(r => r.json());
const target = targets.find(t => t.type === 'page') ?? targets[0];
if (!target) throw new Error('no CDP target');
const ws = new WebSocket(target.webSocketDebuggerUrl);
const message = await new Promise((resolve, reject) => {
  const timeout = setTimeout(() => reject(new Error('cdp timeout')), 10000);
  ws.addEventListener('open', () => ws.send(JSON.stringify({ id: 1, method: 'Runtime.evaluate', params: { expression: expr, returnByValue: true, awaitPromise: true } })));
  ws.addEventListener('message', event => {
    const data = JSON.parse(event.data);
    if (data.id === 1) {
      clearTimeout(timeout);
      resolve(data);
      ws.close();
    }
  });
  ws.addEventListener('error', reject);
});
console.log(JSON.stringify(message.result?.result?.value ?? message.result?.exceptionDetails ?? message, null, 2));
