import { serveStdio } from '@modelcontextprotocol/server/stdio';
import { ImaClient, credentialsFromEnvironment } from './ima.mjs';
import { createServer } from './server.mjs';

try {
  if (Number(process.versions.node.split('.')[0]) < 22) throw new Error('Node 22 required');
  const client = new ImaClient(credentialsFromEnvironment());
  serveStdio(() => createServer(client), {
    onerror: () => { console.error('ima-mcp: MCP transport error'); },
  });
} catch {
  console.error('ima-mcp: 启动失败；需要 Node 22，并通过 mcplink 托管环境提供 IMA_OPENAPI_CLIENTID 和 IMA_OPENAPI_APIKEY。');
  process.exitCode = 1;
}
