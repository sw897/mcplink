import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { Client } from '@modelcontextprotocol/client';
import { StdioClientTransport } from '@modelcontextprotocol/client/stdio';
import { InMemoryTransport } from '@modelcontextprotocol/server';
import { serveStdio } from '@modelcontextprotocol/server/stdio';
import { createServer } from '../src/server.mjs';
import { ImaError } from '../src/ima.mjs';
import { fixture, credentials } from './helpers.mjs';

async function connect(t, ima) {
  const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
  const handle = serveStdio(() => createServer(ima), { transport: serverTransport });
  const client = new Client({ name: 'ima-fixture-client', version: '1.0.0' });
  await client.connect(clientTransport);
  t.after(async () => { await client.close(); await handle.close(); });
  return client;
}

test('MCP exposes only nine read tools, strict schemas and no business request during discovery', async t => {
  const api = await fixture(t);
  const client = await connect(t, api.client);
  const result = await client.listTools();
  assert.equal(result.tools.length, 9);
  assert(!result.tools.some(tool => /append|import|upload|create|delete/.test(tool.name)));
  for (const tool of result.tools) {
    assert.equal(tool.annotations.readOnlyHint, true);
    assert.equal(tool.annotations.destructiveHint, false);
    assert.equal(tool.inputSchema.additionalProperties, false);
  }
  assert.equal(api.requests.length, 0);
  const capabilities = client.getServerCapabilities();
  assert(!capabilities.resources && !capabilities.prompts);
});

test('MCP marks an ima business failure as isError and rejects writes before dispatch', async t => {
  const api = await fixture(t, (_request, response) => response.end(JSON.stringify({ code: 110030,
    msg: `Denied ${credentials.apiKey}`, data: {} })));
  const client = await connect(t, api.client);
  const result = await client.callTool({ name: 'list_notes', arguments: {} });
  assert.equal(result.isError, true);
  assert.equal(result.structuredContent.error.businessCode, 110030);
  assert(!JSON.stringify(result).includes(credentials.apiKey));
  await assert.rejects(client.callTool({ name: 'append_doc', arguments: { content: 'write' } }));
  assert.equal(api.requests.length, 1);
});

test('MCP preserves successful raw data and pagination without automatically requesting another page', async t => {
  const api = await fixture(t, (_request, response) => response.end(JSON.stringify({ code: 0, msg: 'ok',
    data: { is_end: false, next_cursor: 'fixture-next', info_list: [{ id: 'kb-fixture', name: '合成知识库' }] } })));
  const client = await connect(t, api.client);
  const result = await client.callTool({ name: 'search_knowledge_bases', arguments: { limit: 1 } });
  assert.equal(result.isError, false);
  assert.equal(result.structuredContent.data.next_cursor, 'fixture-next');
  assert.equal(result.structuredContent.data.info_list[0].name, '合成知识库');
  assert.equal(api.requests.length, 1);
});

test('MCP cancellation reaches the in-flight ima request', async t => {
  let received, cancelled;
  const started = new Promise(resolve => { received = resolve; });
  const stopped = new Promise(resolve => { cancelled = resolve; });
  const client = await connect(t, { async call(_name, _input, signal) {
    assert(signal instanceof AbortSignal);
    received();
    return new Promise((_resolve, reject) => signal.addEventListener('abort', () => {
      cancelled();
      reject(new ImaError('IMA_CANCELLED', 'cancelled'));
    }, { once: true }));
  } });
  const controller = new AbortController();
  const call = client.callTool({ name: 'list_notes', arguments: {} }, { signal: controller.signal });
  const rejected = assert.rejects(call);
  await started;
  controller.abort();
  await rejected;
  await Promise.race([stopped, new Promise((_resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('MCP cancellation did not reach upstream')), 2000);
    timer.unref();
  })]);
});

test('the production Node 22 stdio entry negotiates MCP and discovers tools without calling ima', async t => {
  const transport = new StdioClientTransport({ command: process.execPath,
    args: [fileURLToPath(new URL('../src/stdio.mjs', import.meta.url))],
    env: { IMA_OPENAPI_CLIENTID: credentials.clientId, IMA_OPENAPI_APIKEY: credentials.apiKey }, stderr: 'pipe' });
  let errors = '';
  transport.stderr.on('data', chunk => { errors += chunk; });
  const client = new Client({ name: 'ima-stdio-fixture', version: '1.0.0' });
  t.after(() => client.close());
  await client.connect(transport);
  const result = await client.listTools();
  assert.equal(result.tools.length, 9);
  assert.equal(errors, '');
});
