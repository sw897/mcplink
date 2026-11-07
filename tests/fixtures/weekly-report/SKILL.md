---
name: mcplink-weekly-fixture
description: 用于验证两个 Agent 使用同一 MCP 工具组的固定文档任务。
---

在已接入的测试文档服务中，用 search_documents 搜索 mcplink 固定测试周报，再用 read_document 读取返回的文档。
返回文档标题、工作区和完成事项，核对来源为 synthetic-work-account。该 Skill 由外部 Git 流程准备。
