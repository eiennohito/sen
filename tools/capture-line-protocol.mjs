// Live LINE Chrome protocol capture through the user-approved Chrome debugger.
// Records request/response SHAPES only: hosts, paths, methods, field names,
// value types, small enum integers and enum-like constants. Strings, IDs,
// tokens, message bodies, media and large numbers are replaced by
// placeholders. Output goes to artifacts/captures/<stamp>/ (ignored by Git).
//
// Control while running: append lines to <run>/control
//   mark <label>   inserts a step marker into the trace
//   shot <label>   saves a screenshot of the LINE page (private, local only)
//   stop           detaches and exits
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

const extension = 'chrome-extension://ophjlpahpchlmihnnnihgmmeilfjmjjc/';
const root = path.resolve(path.dirname(new URL(import.meta.url).pathname), '..');
const stamp = new Date().toISOString().replace(/[:.]/g, '-');
const run = path.join(root, 'artifacts/captures', stamp);
await fs.mkdir(run, { recursive: true, mode: 0o700 });
const trace = await fs.open(path.join(run, 'trace.jsonl'), 'a', 0o600);
const control = path.join(run, 'control');
await fs.writeFile(control, '', { mode: 0o600 });
const write = record => trace.write(JSON.stringify({ t: Date.now(), ...record }) + '\n');

// ---- Shapes ---------------------------------------------------------------
const ID_KEY = /^[a-z][0-9a-f]{32}$|^[A-Za-z0-9+/=_-]{24,}$/;
function shapeString(value) {
  if (/^[A-Z][A-Z0-9_]{1,40}$/.test(value)) return value; // enum constants
  if (/^[A-Za-z0-9_.-]{1,3}$/.test(value)) return value; // tiny codes, e.g. STKOPT
  if (/^-?\d+$/.test(value)) return `<digits:${value.length}>`;
  if (/^[a-z][0-9a-f]{32}$/.test(value)) return `<mid:${value[0]}>`;
  // Links inside payloads are message content: keep the host only.
  if (/^https?:\/\//.test(value)) {
    try { return `<url:${new URL(value).host}>`; } catch { return '<url>'; }
  }
  const trimmed = value.trim();
  if ((trimmed.startsWith('{') || trimmed.startsWith('[')) && trimmed.length < 1 << 20) {
    try { return { '<json>': shape(JSON.parse(trimmed)) }; } catch { /* not JSON */ }
  }
  return `<str:${value.length}>`;
}
function shape(value, depth = 0) {
  if (depth > 24) return '<deep>';
  if (value === null || typeof value === 'boolean') return value;
  if (typeof value === 'number') {
    return Number.isInteger(value) && Math.abs(value) < 1000 ? value : '<num>';
  }
  if (typeof value === 'string') return shapeString(value);
  if (Array.isArray(value)) {
    // Keep the first three distinct item shapes and the length.
    const items = [];
    const seen = new Set();
    for (const item of value) {
      const s = shape(item, depth + 1);
      const k = JSON.stringify(s);
      if (!seen.has(k)) { seen.add(k); items.push(s); }
      if (items.length === 3) break;
    }
    return value.length > items.length ? [...items, `<len:${value.length}>`] : items;
  }
  const out = {};
  for (const [key, item] of Object.entries(value)) {
    out[ID_KEY.test(key) ? '<id-key>' : key] = shape(item, depth + 1);
  }
  return out;
}
function shapeUrl(raw) {
  try {
    const url = new URL(raw);
    // Third-party hosts (link previews, OG images) reveal message content.
    if (!/(^|\.)(line-apps\.com|line-scdn\.net|line\.me|naver\.jp)$/.test(url.host)) return `${url.protocol}//${url.host}/<path>`;
    // Keep word-like segments (thrift service and method names); hide IDs.
    const segments = url.pathname.split('/').map(s =>
      /^[A-Za-z][a-z]*(?:[A-Z0-9][a-z]*)*\d*$/.test(s) && s.length <= 60 && !/\d{4,}/.test(s)
        ? s : s.length <= 12 && !/\d{5,}/.test(s) && !/^[0-9a-f-]{8,}$/.test(s) ? s : '<id>');
    const query = [...url.searchParams.entries()].map(([k, v]) => {
      const key = /^[A-Za-z_]{1,32}$/.test(k) ? k : '<key>';
      const s = shapeString(v);
      return `${key}=${typeof s === 'string' ? s : '<json>'}`;
    });
    return `${url.protocol}//${url.host}${segments.join('/')}${query.length ? '?' + query.join('&') : ''}`;
  } catch { return '<bad-url>'; }
}
const SAFE_HEADERS = new Set(['content-type', 'accept', 'x-line-application', 'x-line-chrome-version', 'x-lal', 'x-lhm', 'x-obs-host', 'x-obs-channeltype']);
function shapeHeaders(headers = {}) {
  const out = {};
  for (const [name, value] of Object.entries(headers)) {
    const key = name.toLowerCase();
    if (SAFE_HEADERS.has(key)) out[key] = String(value).replace(/\d{9,}/g, '<n>');
    else if (/params/.test(key)) {
      try { out[key] = { '<base64-json>': shape(JSON.parse(Buffer.from(value, 'base64').toString('utf8'))) }; }
      catch { out[key] = shapeString(String(value)); }
    } else out[key] = '<redacted>';
  }
  return out;
}
const lineApi = url => /(^|\.)line-apps\.com$|(^|\.)line\.me$|(^|\.)naver\.jp$/.test(new URL(url).host);
function shapeBody(text, url) {
  if (text == null) return undefined;
  if (!lineApi(url)) return `<body:${text.length}>`;
  try { return shape(JSON.parse(text)); } catch { return `<text:${text.length}>`; }
}

// ---- CDP ------------------------------------------------------------------
const endpoint = path.join(os.homedir(), '.config/google-chrome/DevToolsActivePort');
const [port, route] = (await fs.readFile(endpoint, 'utf8')).trim().split('\n');
const socket = new WebSocket(`ws://127.0.0.1:${port}${route}`);
await new Promise((resolve, reject) => {
  const timer = setTimeout(() => reject(new Error('Chrome connection timed out; check its debugger approval prompt')), 60000);
  socket.onopen = () => { clearTimeout(timer); resolve(); };
  socket.onerror = () => { clearTimeout(timer); reject(new Error('Chrome connection failed')); };
});
let sequence = 0;
const pending = new Map();
const handlers = [];
socket.onmessage = event => {
  const message = JSON.parse(event.data);
  if (message.id) {
    const request = pending.get(message.id);
    if (!request) return;
    pending.delete(message.id);
    message.error ? request.reject(new Error(message.error.message)) : request.resolve(message.result);
  } else for (const handler of handlers) handler(message);
};
function call(method, params = {}, sessionId) {
  const id = ++sequence;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`CDP timed out: ${method}`)); }, 15000);
    pending.set(id, {
      resolve: r => { clearTimeout(timer); resolve(r); },
      reject: e => { clearTimeout(timer); reject(e); },
    });
    socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
  });
}

const sessions = new Map(); // sessionId -> { kind, page }
const requests = new Map(); // `${sessionId}:${requestId}` -> info
const streams = new Map();  // key -> buffered SSE text
let counts = { requests: 0, events: 0 };

handlers.push(async ({ method, params, sessionId }) => {
  if (!sessions.has(sessionId)) return;
  const key = `${sessionId}:${params?.requestId}`;
  try {
    if (method === 'Network.requestWillBeSent') {
      const { request } = params;
      if (!/^https?:/.test(request.url)) return;
      requests.set(key, { url: request.url });
      counts.requests++;
      write({
        kind: 'request', target: sessions.get(sessionId).kind, id: params.requestId,
        method: request.method, url: shapeUrl(request.url), headers: shapeHeaders(request.headers),
        body: request.hasPostData && request.postData === undefined
          ? '<body-not-inlined>' : shapeBody(request.postData, request.url),
        bodyBytes: request.postData?.length,
      });
    } else if (method === 'Network.responseReceived') {
      const info = requests.get(key);
      if (!info) return;
      info.mime = params.response.mimeType;
      write({ kind: 'response', id: params.requestId, status: params.response.status, mime: info.mime, url: shapeUrl(info.url) });
      if (info.mime === 'text/event-stream' || /\/api\/operation\/receive/.test(info.url)) {
        streams.set(key, '');
        const { bufferedData } = await call('Network.streamResourceContent', { requestId: params.requestId }, sessionId);
        if (bufferedData) sse(key, Buffer.from(bufferedData, 'base64').toString('utf8'));
      }
    } else if (method === 'Network.dataReceived' && streams.has(key) && params.data) {
      sse(key, Buffer.from(params.data, 'base64').toString('utf8'));
    } else if (method === 'Network.loadingFinished') {
      const info = requests.get(key);
      requests.delete(key);
      if (!info || streams.has(key)) { streams.delete(key); return; }
      if (!/json|text|javascript/.test(info.mime ?? '')) {
        write({ kind: 'body', id: params.requestId, bytes: params.encodedDataLength, note: info.mime });
        return;
      }
      const { body, base64Encoded } = await call('Network.getResponseBody', { requestId: params.requestId }, sessionId);
      write({ kind: 'body', id: params.requestId, body: shapeBody(base64Encoded ? '' : body, info.url) });
    } else if (method === 'Network.loadingFailed') {
      requests.delete(key);
      write({ kind: 'failed', id: params.requestId, error: params.errorText, canceled: params.canceled });
    }
  } catch (error) {
    write({ kind: 'capture-error', id: params?.requestId, error: String(error.message) });
  }
});
function sse(key, text) {
  const buffer = streams.get(key) + text;
  const parts = buffer.split(/\r?\n\r?\n/);
  streams.set(key, parts.pop());
  for (const part of parts) {
    let event = 'message';
    const data = [];
    for (const line of part.split(/\r?\n/)) {
      if (line.startsWith('event:')) event = line.slice(6).trim();
      else if (line.startsWith('data:')) data.push(line.slice(5).trimStart());
    }
    if (!data.length) continue;
    counts.events++;
    let payload;
    try { payload = shape(JSON.parse(data.join('\n'))); } catch { payload = `<text:${data.join('\n').length}>`; }
    write({ kind: 'sse', event, data: payload });
  }
}

async function attachNew() {
  const { targetInfos } = await call('Target.getTargets');
  for (const target of targetInfos) {
    if (!target.url.startsWith(extension)) continue;
    if ([...sessions.values()].some(s => s.targetId === target.targetId)) continue;
    if (!['page', 'service_worker', 'worker', 'iframe'].includes(target.type)) continue;
    try {
      const { sessionId } = await call('Target.attachToTarget', { targetId: target.targetId, flatten: true });
      const kind = `${target.type}:${new URL(target.url).pathname.split('/').pop() || 'root'}`;
      sessions.set(sessionId, { kind, targetId: target.targetId, page: target.type === 'page' });
      await call('Network.enable', { maxPostDataSize: 1 << 20 }, sessionId);
      if (target.type !== 'page') await call('Runtime.runIfWaitingForDebugger', {}, sessionId).catch(() => {});
      write({ kind: 'attached', target: kind });
      console.log(`attached ${kind}`);
    } catch (error) {
      write({ kind: 'attach-error', target: target.type, error: String(error.message) });
    }
  }
}

let offset = 0;
let running = true;
async function readControl() {
  const text = await fs.readFile(control, 'utf8');
  const lines = text.slice(offset).split('\n');
  offset = text.length - lines.pop().length;
  for (const line of lines.map(l => l.trim()).filter(Boolean)) {
    const [command, ...rest] = line.split(' ');
    const label = rest.join(' ');
    if (command === 'mark') { write({ kind: 'mark', label }); console.log(`mark ${label}`); }
    else if (command === 'stop') running = false;
    else if (command === 'shot') {
      for (const [sessionId, session] of sessions) {
        if (!session.page) continue;
        try {
          const { data } = await call('Page.captureScreenshot', { format: 'png' }, sessionId);
          const file = path.join(run, `${label.replace(/[^\w.-]+/g, '_')}-${session.kind.replace(/\W+/g, '_')}.png`);
          await fs.writeFile(file, Buffer.from(data, 'base64'), { mode: 0o600 });
          write({ kind: 'shot', label, file: path.basename(file) });
          console.log(`shot ${file}`);
        } catch (error) { write({ kind: 'shot-error', label, error: String(error.message) }); }
      }
    }
  }
}

console.log(`capture ${run}`);
write({ kind: 'start' });
try {
  while (running) {
    await attachNew();
    await readControl();
    await new Promise(r => setTimeout(r, 500));
  }
} finally {
  write({ kind: 'stop', ...counts });
  for (const sessionId of sessions.keys()) await call('Target.detachFromTarget', { sessionId }).catch(() => {});
  await trace.close();
  socket.close();
  console.log(`stopped: ${counts.requests} requests, ${counts.events} stream events`);
}
