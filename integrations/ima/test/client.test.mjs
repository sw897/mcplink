import test from 'node:test';
import assert from 'node:assert/strict';
import { ImaClient, ImaError, credentialsFromEnvironment, CONTRACT_VERSION } from '../src/ima.mjs';
import { fixture, credentials } from './helpers.mjs';

test('documented read operations preserve IDs, cursor types, Unicode and required defaults', async t => {
  const api = await fixture(t);
  const cases = [
    ['search_knowledge_bases', { query: '产品资料', cursor: 'opaque/游标+=' }, '/wiki/v1/search_knowledge_base', { query: '产品资料', cursor: 'opaque/游标+=', limit: 20 }],
    ['get_knowledge_bases', { ids: ['kb-opaque'] }, '/wiki/v1/get_knowledge_base', { ids: ['kb-opaque'] }],
    ['list_knowledge', { knowledge_base_id: 'kb-opaque', folder_id: 'folder-opaque' }, '/wiki/v1/get_knowledge_list', { knowledge_base_id: 'kb-opaque', folder_id: 'folder-opaque', cursor: '', limit: 20 }],
    ['search_knowledge', { knowledge_base_id: 'kb-opaque', query: '会议' }, '/wiki/v1/search_knowledge', { knowledge_base_id: 'kb-opaque', query: '会议', cursor: '' }],
    ['get_media_info', { media_id: 'media-opaque' }, '/wiki/v1/get_media_info', { media_id: 'media-opaque' }],
    ['list_notebooks', {}, '/note/v1/list_notebook', { cursor: '0', limit: 20 }],
    ['list_notes', {}, '/note/v1/list_note', { sort_type: 0, cursor: '', limit: 20 }],
    ['search_notes', { query: '实施方案', search_in: 'content', start: 20, limit: 10 }, '/note/v1/search_note', { search_type: 1, query_info: { content: '实施方案' }, start: 20, end: 30, sort_type: 0 }],
    ['get_note_content', { note_id: 'note-opaque' }, '/note/v1/get_doc_content', { note_id: 'note-opaque', target_content_format: 0 }],
  ];
  for (const [name, input, path, body] of cases) {
    await api.client.call(name, input);
    const request = api.requests.at(-1);
    assert.equal(request.path, `/openapi${path}`);
    assert.equal(request.method, 'POST');
    assert.deepEqual(request.body, body);
    assert.equal(request.headers['ima-openapi-clientid'], credentials.clientId);
    assert.equal(request.headers['ima-openapi-apikey'], credentials.apiKey);
    assert.equal(request.headers['ima-openapi-ctx'], `skill_version=${CONTRACT_VERSION}`);
  }
  assert.equal(api.requests.length, cases.length);
});

test('writes, arbitrary paths, extra headers and invalid paging fail before HTTP', async t => {
  const api = await fixture(t);
  for (const [name, input] of [
    ['append_doc', { note_id: 'note', content: 'write' }],
    ['/openapi/wiki/v1/create_media', {}],
    ['get_note_content', { note_id: 'note', target_content_format: 1 }],
    ['search_knowledge_bases', { url: 'https://other.example', headers: {} }],
    ['get_knowledge_bases', { ids: ['duplicate', 'duplicate'] }],
    ['list_notes', { limit: 21 }],
    ['list_knowledge', { knowledge_base_id: 'kb', cursor: 1 }],
    ['search_notes', { query: 'term', limit: 21 }],
  ]) await assert.rejects(api.client.call(name, input), ImaError);
  assert.equal(api.requests.length, 0);
});

test('HTTP 200 business failure remains a failure and credentials are redacted', async t => {
  const api = await fixture(t, (_request, response) => response.end(JSON.stringify({
    code: 110030, msg: `Denied ${credentials.clientId} ${credentials.apiKey}`, data: {},
  })));
  await assert.rejects(api.client.call('list_notes'), error => {
    assert.equal(error.code, 'IMA_BUSINESS_ERROR');
    assert.equal(error.details.businessCode, 110030);
    assert(!error.message.includes(credentials.clientId));
    assert(!error.message.includes(credentials.apiKey));
    return true;
  });
  assert.equal(api.requests.length, 1);
});

test('successful data keeps pagination and hides credentials accidentally echoed upstream', async t => {
  const api = await fixture(t, (_request, response) => response.end(JSON.stringify({ code: 0, msg: 'ok',
    data: { is_end: false, next_cursor: 'opaque-next', content: '合成正文', nested: [credentials.apiKey, credentials.clientId] } })));
  const result = await api.client.call('list_notes');
  assert.equal(result.data.next_cursor, 'opaque-next');
  assert.equal(result.data.is_end, false);
  assert.equal(result.data.content, '合成正文');
  assert(!JSON.stringify(result).includes(credentials.apiKey));
  assert(!JSON.stringify(result).includes(credentials.clientId));
});

test('redirects and HTTP failures do not follow or retry and do not echo response bodies', async t => {
  const api = await fixture(t, (_request, response) => {
    response.statusCode = 302;
    response.setHeader('location', '/unexpected');
    response.end(credentials.apiKey);
  });
  await assert.rejects(api.client.call('list_notes'), error => error.code === 'IMA_HTTP_ERROR' && error.details.httpStatus === 302 && !error.message.includes(credentials.apiKey));
  assert.equal(api.requests.length, 1);
});

test('malformed envelopes are rejected rather than reported as success', async t => {
  for (const body of ['not JSON', '{"data":{}}', '{"code":"0","data":{}}', '{"code":0}']) {
    const api = await fixture(t, (_request, response) => response.end(body));
    await assert.rejects(api.client.call('list_notes'), error => error.code === 'IMA_RESPONSE_INVALID');
  }
});

test('declared and streamed oversized responses stop at the configured bound', async t => {
  for (const declared of [true, false]) {
    const api = await fixture(t, (_request, response) => {
      if (declared) response.setHeader('content-length', '256');
      response.write('x'.repeat(256));
      response.end();
    });
    const client = new ImaClient({ ...credentials, origin: api.origin, allowLoopbackForTests: true, maxResponseBytes: 128 });
    await assert.rejects(client.call('list_notes'), error => error.code === 'IMA_RESPONSE_TOO_LARGE');
  }
});

test('timeout and explicit cancellation end a single request', async t => {
  let received;
  let count = 0;
  const pending = new Promise(resolve => { received = resolve; });
  const api = await fixture(t, () => { if (++count === 2) received(); });
  const timed = new ImaClient({ ...credentials, origin: api.origin, allowLoopbackForTests: true, timeoutMs: 80 });
  await assert.rejects(timed.call('list_notes'), error => error.code === 'IMA_TIMEOUT');
  const controller = new AbortController();
  const request = api.client.call('list_notes', {}, controller.signal);
  await pending;
  controller.abort();
  await assert.rejects(request, error => error.code === 'IMA_CANCELLED');
  assert.equal(api.requests.length, 2);
});

test('production credentials have no implicit file fallback or origin override', () => {
  assert.deepEqual(credentialsFromEnvironment({ IMA_CLIENT_ID: 'ignored', IMA_API_KEY: 'ignored' }),
    { clientId: undefined, apiKey: undefined });
  assert.throws(() => new ImaClient({}), error => error.code === 'IMA_CREDENTIALS_MISSING');
  for (const origin of ['https://other.example', 'http://127.0.0.1:1', 'https://ima.qq.com/path', 'https://x:password@ima.qq.com']) {
    assert.throws(() => new ImaClient({ ...credentials, origin }), error => error.code === 'IMA_CONFIG_INVALID');
  }
});
