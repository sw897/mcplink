import * as z from 'zod/v4';

const id = () => z.string().min(1).max(4096).refine(value => value.trim().length > 0);
const cursor = (initial = '') => z.string().max(8192).default(initial);
const limit = (maximum) => z.number().int().min(1).max(maximum).default(20);
const sort = () => z.number().int().min(0).max(3).default(0);

// This is the complete dispatch table. No tool accepts an API path, origin,
// authentication header, or write operation supplied by a caller.
export const operations = Object.freeze([
  {
    name: 'search_knowledge_bases',
    path: '/openapi/wiki/v1/search_knowledge_base',
    description: '按关键词查找 ima 知识库；query 为空时列出当前账号可见的知识库。保留 next_cursor 和 is_end 供后续翻页。',
    schema: z.object({ query: z.string().max(1024).default(''), cursor: cursor(), limit: limit(20) }).strict(),
  },
  {
    name: 'get_knowledge_bases',
    path: '/openapi/wiki/v1/get_knowledge_base',
    description: '读取已从列表取得的 ima 知识库 ID 的详细信息，一次最多 20 个。',
    schema: z.object({ ids: z.array(id()).min(1).max(20).refine(ids => new Set(ids).size === ids.length) }).strict(),
  },
  {
    name: 'list_knowledge',
    path: '/openapi/wiki/v1/get_knowledge_list',
    description: '浏览指定 ima 知识库的根目录或文件夹；文件夹与知识条目一起返回，不自动遍历所有目录或页面。',
    schema: z.object({ knowledge_base_id: id(), folder_id: id().optional(), cursor: cursor(), limit: limit(50) }).strict(),
  },
  {
    name: 'search_knowledge',
    path: '/openapi/wiki/v1/search_knowledge',
    description: '在已选定的 ima 知识库内搜索关键词，返回条目 ID、标题和命中片段；不把片段当作完整原文。',
    schema: z.object({ knowledge_base_id: id(), query: z.string().min(1).max(1024), cursor: cursor() }).strict(),
  },
  {
    name: 'get_media_info',
    path: '/openapi/wiki/v1/get_media_info',
    description: '取得知识条目的原文访问信息。media_type=11 时用 notebook_ext_info.notebook_id 调用 get_note_content；其他类型返回授权访问链接及其必要请求头，本服务不下载外部文件。',
    schema: z.object({ media_id: id() }).strict(),
  },
  {
    name: 'list_notebooks',
    path: '/openapi/note/v1/list_notebook',
    description: '列出 ima 笔记本；首个 cursor 为字符串 0，后续使用返回的 next_cursor。version 仅使用服务端返回的值。',
    schema: z.object({ cursor: cursor('0'), limit: limit(20), version: z.string().max(8192).optional() }).strict(),
  },
  {
    name: 'list_notes',
    path: '/openapi/note/v1/list_note',
    description: '列出当前账号的 ima 笔记，可按已有 folder_id 缩小到指定笔记本。保留原有分页字段。',
    schema: z.object({ folder_id: id().optional(), sort_type: sort(), cursor: cursor(), limit: limit(20) }).strict(),
  },
  {
    name: 'search_notes',
    path: '/openapi/note/v1/search_note',
    description: '按标题或正文关键词搜索 ima 笔记。start 是结果起始编号，limit 最多 20；is_end=true 时停止翻页。',
    schema: z.object({ query: z.string().min(1).max(1024), search_in: z.enum(['title', 'content']).default('title'),
      start: z.number().int().min(0).max(2_147_483_627).default(0), limit: limit(20), sort_type: sort() }).strict(),
    body: ({ query, search_in, start, limit, sort_type }) => ({
      search_type: search_in === 'title' ? 0 : 1,
      query_info: { [search_in]: query }, start, end: start + limit, sort_type,
    }),
  },
  {
    name: 'get_note_content',
    path: '/openapi/note/v1/get_doc_content',
    description: '读取用户有权访问的 ima 笔记正文。0 为纯文本、2 为 JSON；官方当前不支持格式 1 的 Markdown 正文读取。',
    schema: z.object({ note_id: id(), target_content_format: z.union([z.literal(0), z.literal(2)]).default(0) }).strict(),
  },
]);

export const annotations = Object.freeze({ readOnlyHint: true, destructiveHint: false,
  idempotentHint: true, openWorldHint: true });
