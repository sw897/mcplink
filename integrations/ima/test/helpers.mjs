import { createServer as createHttpServer } from 'node:http';
import { ImaClient } from '../src/ima.mjs';

export const credentials = { clientId: 'synthetic-client-id', apiKey: 'synthetic-api-key' };

export async function fixture(t, handler = (_request, response) => {
  response.setHeader('content-type', 'application/json');
  response.end(JSON.stringify({ code: 0, msg: 'success', data: { is_end: true } }));
}) {
  const requests = [];
  const server = createHttpServer(async (request, response) => {
    const buffers = [];
    for await (const chunk of request) buffers.push(chunk);
    const observed = { path: request.url, headers: request.headers, method: request.method,
      body: JSON.parse(Buffer.concat(buffers).toString('utf8')) };
    requests.push(observed);
    await handler(observed, response);
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  t.after(() => new Promise(resolve => { server.closeAllConnections(); server.close(resolve); }));
  const origin = `http://127.0.0.1:${server.address().port}`;
  return { requests, origin, client: new ImaClient({ ...credentials, origin, allowLoopbackForTests: true }) };
}
