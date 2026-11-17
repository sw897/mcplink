import { operations } from './operations.mjs';

export const CONTRACT_VERSION = '1.1.10';
export const IMA_ORIGIN = 'https://ima.qq.com';
const MAX_RESPONSE_BYTES = 4 * 1024 * 1024;

export class ImaError extends Error {
  constructor(code, message, details = {}) {
    super(message);
    this.code = code;
    this.details = details;
  }
}

export class ImaClient {
  #clientId;
  #apiKey;
  #origin;
  #timeout;
  #maxResponse;

  constructor({ clientId, apiKey, origin = IMA_ORIGIN, allowLoopbackForTests = false,
    timeoutMs = 30_000, maxResponseBytes = MAX_RESPONSE_BYTES } = {}) {
    for (const value of [clientId, apiKey]) {
      if (typeof value !== 'string' || !value.trim() || value.length > 65536 || /[\r\n\0]/.test(value)) {
        throw new ImaError('IMA_CREDENTIALS_MISSING', '请通过 mcplink 的托管环境提供 ima Client ID 和 API Key。');
      }
    }
    let url;
    try { url = new URL(origin); } catch { throw new ImaError('IMA_CONFIG_INVALID', 'ima 服务地址无效。'); }
    const official = url.origin === IMA_ORIGIN;
    const fixture = allowLoopbackForTests && url.protocol === 'http:' && url.hostname === '127.0.0.1';
    if ((!official && !fixture) || url.username || url.password || url.search || url.hash || url.pathname !== '/') {
      throw new ImaError('IMA_CONFIG_INVALID', 'ima 请求只允许发送到官方源；测试仅允许显式注入本机回环地址。');
    }
    if (!Number.isInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 120_000 ||
        !Number.isInteger(maxResponseBytes) || maxResponseBytes < 1 || maxResponseBytes > MAX_RESPONSE_BYTES) {
      throw new ImaError('IMA_CONFIG_INVALID', 'ima 请求限制无效。');
    }
    this.#clientId = clientId;
    this.#apiKey = apiKey;
    this.#origin = url.origin;
    this.#timeout = timeoutMs;
    this.#maxResponse = maxResponseBytes;
  }

  #redact(value, depth = 0) {
    if (depth > 100) throw new ImaError('IMA_RESPONSE_INVALID', 'ima 返回的 JSON 嵌套过深。');
    if (typeof value === 'string') {
      for (const secret of [this.#clientId, this.#apiKey]) {
        value = value.replaceAll(secret, '[redacted]').replaceAll(encodeURIComponent(secret), '[redacted]');
      }
      return value;
    }
    if (Array.isArray(value)) return value.map(item => this.#redact(item, depth + 1));
    if (value && typeof value === 'object') {
      return Object.fromEntries(Object.entries(value).map(([key, item]) =>
        [this.#redact(key, depth + 1), this.#redact(item, depth + 1)]));
    }
    return value;
  }

  async call(name, input = {}, signal) {
    const operation = operations.find(item => item.name === name);
    if (!operation) throw new ImaError('IMA_TOOL_NOT_ALLOWED', '此 ima 服务只开放已登记的只读工具。');
    const parsed = operation.schema.safeParse(input);
    if (!parsed.success) throw new ImaError('IMA_INPUT_INVALID', '工具参数不符合 ima 只读接口定义，请核对工具 Schema。');
    const body = operation.body ? operation.body(parsed.data) : parsed.data;
    const timeout = AbortSignal.timeout(this.#timeout);
    const combined = signal ? AbortSignal.any([timeout, signal]) : timeout;
    let response;
    try {
      response = await fetch(new URL(operation.path, this.#origin), {
        method: 'POST', redirect: 'manual', signal: combined,
        headers: { 'content-type': 'application/json', 'ima-openapi-clientid': this.#clientId,
          'ima-openapi-apikey': this.#apiKey, 'ima-openapi-ctx': `skill_version=${CONTRACT_VERSION}`,
          'user-agent': 'mcplink-ima/0.1.0' },
        body: JSON.stringify(body),
      });
      if (!response.ok) {
        await response.body?.cancel();
        throw new ImaError('IMA_HTTP_ERROR', `ima 返回 HTTP ${response.status}，请求未自动重试。`, { httpStatus: response.status });
      }
      const declared = Number(response.headers.get('content-length'));
      if (declared > this.#maxResponse) {
        await response.body?.cancel();
        throw new ImaError('IMA_RESPONSE_TOO_LARGE', 'ima 响应超过 4 MiB 上限，请缩小查询范围。');
      }
      const reader = response.body?.getReader();
      if (!reader) throw new ImaError('IMA_RESPONSE_INVALID', 'ima 未返回响应正文。');
      const chunks = [];
      let bytes = 0;
      try {
        while (true) {
          const part = await reader.read();
          if (part.done) break;
          bytes += part.value.byteLength;
          if (bytes > this.#maxResponse) {
            await reader.cancel();
            throw new ImaError('IMA_RESPONSE_TOO_LARGE', 'ima 响应超过 4 MiB 上限，请缩小查询范围。');
          }
          chunks.push(Buffer.from(part.value));
        }
      } finally { reader.releaseLock(); }
      let envelope;
      try { envelope = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(Buffer.concat(chunks))); }
      catch { throw new ImaError('IMA_RESPONSE_INVALID', 'ima 未返回有效的 UTF-8 JSON。'); }
      if (!envelope || typeof envelope !== 'object' || Array.isArray(envelope) || !Number.isSafeInteger(envelope.code)) {
        throw new ImaError('IMA_RESPONSE_INVALID', 'ima 响应缺少有效业务状态码。');
      }
      if (envelope.code !== 0) {
        const message = typeof envelope.msg === 'string' ? this.#redact(envelope.msg).replace(/[\x00-\x1f\x7f]/g, ' ').slice(0, 512) : 'ima 拒绝了请求，请核对授权和参数。';
        throw new ImaError('IMA_BUSINESS_ERROR', message, { businessCode: envelope.code });
      }
      if (!Object.hasOwn(envelope, 'data')) throw new ImaError('IMA_RESPONSE_INVALID', 'ima 成功响应缺少 data 字段。');
      return this.#redact(envelope);
    } catch (error) {
      if (error instanceof ImaError) throw error;
      if (timeout.aborted) throw new ImaError('IMA_TIMEOUT', 'ima 请求超时，未自动重试。');
      if (signal?.aborted) throw new ImaError('IMA_CANCELLED', 'ima 请求已取消。');
      throw new ImaError('IMA_NETWORK_ERROR', 'ima 通信失败，未自动重试；请核对网络后再决定是否重试。');
    }
  }
}

export function credentialsFromEnvironment(environment = process.env) {
  return { clientId: environment.IMA_OPENAPI_CLIENTID, apiKey: environment.IMA_OPENAPI_APIKEY };
}
