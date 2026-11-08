# 参与开发

mcplink 使用 React/TypeScript、Tauri 和 Rust。请先阅读 [README](README.md)、[架构](docs/architecture.md)及[兼容范围](docs/compatibility.md)。

## 开发环境

- 使用 `.node-version` 和 `rust-toolchain.toml` 中的工具链。
- Python 3.11+ 运行端到端脚本。
- 安装目标平台所需的 Tauri 原生构建工具。

```sh
npm ci
npm run desktop
```

## 验证修改

```sh
npm run build
npm run check
python3 tests/e2e/run.py
python3 tests/e2e/agent_cli.py --record artifacts/e2e/agent-cli/local
python3 tests/e2e/claude_sources.py --record artifacts/e2e/claude/local
python3 tests/e2e/tool_calls.py --record artifacts/e2e/tool-calls/local
```

检查命令会先构建并同步 debug sidecar。隔离测试使用合成 home、MCP 上游和 fixture Vault。`MCPLINK_E2E_BINARY` 可指定待测程序；每次测试应使用新的记录目录。

`run.py --native-agents` 额外调用已安装的 Codex/Hermes 命令，需要对应客户端环境。默认测试不要求模型登录或真实服务账号。平台验证结果应分别记录；CI 构建通过不等于桌面交互已验收。

配置、凭据或后台行为的改动应覆盖实际 CLI/Bridge 路径、失败恢复和无关设置保留。新增依赖需说明用途、许可，并更新锁文件及第三方声明。生成声明前需安装锁定的依赖：

```sh
python3 scripts/third-party-notices.py
```

## 提交问题或修改

请说明复现步骤、预期与实际结果、应用/Agent 版本和操作系统。提交修改时概述问题、行为变化及实际验证结果；没有运行的检查应明确标注。

不要附上真实凭据、原生配置、数据库或文档正文。请用合成的最小样例描述配置问题，安全问题按 [SECURITY.md](SECURITY.md) 处理。
