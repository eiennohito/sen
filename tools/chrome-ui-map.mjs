// Local CDP inspector. Default output omits account content; custom expressions
// are unrestricted and must be reviewed for privacy and side effects.
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

const extension = 'chrome-extension://ophjlpahpchlmihnnnihgmmeilfjmjjc/';
const endpoint = path.join(os.homedir(), '.config/google-chrome/DevToolsActivePort');
const [port, route] = (await fs.readFile(endpoint, 'utf8')).trim().split('\n');
const socket = new WebSocket(`ws://127.0.0.1:${port}${route}`);
await new Promise((resolve, reject) => {
  const timer = setTimeout(() => {
    socket.close();
    reject(new Error('Chrome connection timed out; check its debugger approval prompt'));
  }, 60000);
  socket.onopen = () => { clearTimeout(timer); resolve(); };
  socket.onerror = () => { clearTimeout(timer); reject(new Error('Chrome connection failed')); };
});

let sequence = 0;
const pending = new Map();
socket.onmessage = event => {
  const message = JSON.parse(event.data);
  const request = pending.get(message.id);
  if (!request) return;
  pending.delete(message.id);
  if (message.error) request.reject(new Error(message.error.message));
  else request.resolve(message.result);
};
function call(method, params = {}, sessionId) {
  const id = ++sequence;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error(`CDP timed out: ${method}`));
    }, 15000);
    pending.set(id, {
      resolve: result => { clearTimeout(timer); resolve(result); },
      reject: error => { clearTimeout(timer); reject(error); },
    });
    socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
  });
}
const defaultExpression = `JSON.stringify({
  viewport: {width:innerWidth,height:innerHeight,scale:devicePixelRatio},
  modules: [...new Set([...document.querySelectorAll('[class]')].flatMap(e =>
    [...String(e.className).matchAll(/([A-Za-z][A-Za-z0-9]+)-module__/g)].map(m=>m[1])))],
  navigation: [...document.querySelectorAll('[class*="gnb-module"]')]
    .filter(e=>e.tagName==='BUTTON'||e.tagName==='A')
    .map(e=>({tag:e.tagName,role:e.getAttribute('role'),title:e.getAttribute('title'),
      label:e.getAttribute('aria-label'),class:e.className}))
})`;

try {
  const targets = await call('Target.getTargets');
  const target = targets.targetInfos.find(t => t.type === 'page' && t.url.startsWith(`${extension}index.html`));
  if (!target) throw new Error('Logged-in LINE extension page not found');
  let { sessionId } = await call('Target.attachToTarget', { targetId: target.targetId, flatten: true });
  const detach = () => call('Target.detachFromTarget', { sessionId }).catch(() => {});
  const evaluate = async expression => {
    const result = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true }, sessionId);
    console.log(result.exceptionDetails ? 'Inspection expression failed' :
      typeof result.result.value === 'string' ? result.result.value : JSON.stringify(result.result.value));
  };
  if (process.argv[2] === '--interactive') {
    const readline = await import('node:readline');
    console.log('CDP attached to LINE. One inspection expression per line; .exit detaches.');
    for await (const expression of readline.createInterface({ input: process.stdin })) {
      if (expression === '.exit') break;
      try {
        if (expression === ':targets') {
          const targets = await call('Target.getTargets');
          console.log(JSON.stringify(targets.targetInfos
            .filter(t => t.type === 'page' && t.url.startsWith(extension))
            .map(t => ({ id: t.targetId, route: new URL(t.url).hash.split('/').slice(0, 2).join('/') }))));
        } else if (expression.startsWith(':attach ')) {
          await detach();
          ({ sessionId } = await call('Target.attachToTarget', { targetId: expression.slice(8).trim(), flatten: true }));
          console.log('Attached to selected window.');
        } else {
          await evaluate(expression);
        }
      } catch (error) {
        console.log(error.message);
      }
    }
  } else {
    await evaluate(process.argv[2] || defaultExpression);
  }
  await detach();
} finally {
  socket.close();
}
