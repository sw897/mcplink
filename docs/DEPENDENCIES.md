# 依赖与许可

mcplink 自有代码使用 MIT。具体依赖解析由 Cargo.lock 与 package-lock.json 固定；分发应保留第三方许可。主要直接依赖如下。

| 依赖 | 锁定版本 | 用途 | 许可 |
| --- | --- | --- | --- |
| Tauri / tauri-build / dialog plugin | 2.12.5 / 2.7.4 / 2.8.4 | 原生窗口、打包、文件选择 | MIT / Apache-2.0 |
| React / React DOM | 19.3.1 | 桌面界面 | MIT |
| rmcp | 3.9.0 | MCP 客户端/服务端、传输和 OAuth | Apache-2.0 |
| jsonschema | 0.60.2 | 调用工具前校验输入 Schema；禁用 HTTP/文件引用获取 | MIT |
| jsonc-parser | 0.34.0 | JSON/JSONC 校验及保留注释、空白的配置编辑 | MIT |
| tokio / axum | 1.54.1 / 0.8.9 | 异步运行时、本地 HTTP | MIT |
| reqwest | 0.13.5 | rustls HTTP 客户端 | MIT / Apache-2.0 |
| rusqlite | 0.41.1 | SQLite，内嵌 SQLite 3.54.1 | MIT；SQLite public domain |
| keyring | 4.2.0 | 系统凭据库 | MIT / Apache-2.0 |
| windows-sys | 0.61.2 | Windows 后台进程启动与父进程句柄隔离 | MIT / Apache-2.0 |
| toml / toml_edit | 0.9.12 / 0.23.10 | TOML 读取和保留格式编辑 | MIT / Apache-2.0 |
| serde_yaml / yaml-edit | 0.9.34 / 0.3.6 | YAML 读取和保留格式编辑 | MIT / Apache-2.0；Apache-2.0 |
| serde / serde_json | 1.0.229 / 1.0.154 | 序列化 | MIT / Apache-2.0 |
| anyhow / async-trait / base64 | 1.0.104 / 0.1.92 / 0.22.1 | 错误、异步接口、编码 | MIT / Apache-2.0 |
| clap / directories / fs2 | 4.6.7 / 6.0.0 / 0.4.3 | CLI、目录、文件锁 | MIT / Apache-2.0 |
| glob / opener / url | 0.3.4 / 0.8.5 / 2.5.8 | 工具筛选、系统浏览器、地址解析 | MIT / Apache-2.0 |
| sha2 / uuid | 0.10.9 / 1.29.1 | 配置哈希、实例与连接 ID | MIT / Apache-2.0 |
| lucide-react | 1.70.0 | 界面图标 | ISC |
| Inter variable font | 5.3.0 | 本地字体 | OFL-1.1 |
| Vite / TypeScript | 7.3.7 / 5.9.3 | 前端构建、类型检查 | MIT；Apache-2.0 |

`serde_yaml` 已标记 deprecated，当前用于只读解析；YAML 修改由 yaml-edit 完成。后续升级需要重新运行配置保存与恢复场景。SDK 和原生 Agent 协议同时演进，升级应先跑端到端兼容场景。
