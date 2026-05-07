---
doc_type: brainstorm
slug: usage-monitor
created: 2026-05-07
status: active
summary: AI 编码工具 Token 用量监控——双数据源 + 协议归一化 + Web/TUI 双模，参考 Tokscale 架构
tags: [token-usage, ai-coding-tools, monitoring, tui, web-dashboard]
---

# UsageMonitor

> 创意空间 | 2026-05-07 | 下一步：cs-roadmap

## 出发点

使用 Claude Code / Codex / Kimi Code / Gemini CLI / Pi / OpenClaw / Hermess 等多个 AI 编码工具，各家 token 用量散落在各自的 session 日志里，协议不同（Anthropic vs OpenAI）、格式各异，看不到统一的用量和花费。CC Switch 代理了 Claude Code / Codex 的请求并做了结构化存储，但 Kimi Code 等直连工具覆盖不到。

想做一个本地优先的用量监控工具，统一展示所有 AI 编码工具的 token 消耗和花费。

## 聊过的方向

- **数据采集**：讨论了代理拦截 vs 日志解析 vs 混合。结论：双后端——CC Switch 适配器（读已有 DB）+ 原生文件扫描器（Tokscale 式考古），输出同一种 UnifiedMessage
- **技术栈**：对比了 Node.js + React（TokenTracker）、Rust（Tokscale/Scopeon）、Python（TokBurn）。不绑定语言偏好，以架构清晰优先
- **界面**：Web vs TUI vs GUI。结论：Web 主打（仪表盘/历史趋势）+ TUI 辅打（`htop` 式快开快看），不做原生 GUI
- **参考项目**：深入分析了 Tokscale 的源码架构——scanner → per-tool parser → UnifiedMessage → aggregator 的流水线、SIMD JSON 解析、LiteLLM+OpenRouter 定价引擎、ratatui TUI

## 当前倾向

倾向参考 Tokscale 的架构模式（归一化消息模型 + 每工具独立 parser），但要有自己的风格——不做纯 TUI，而是 Web 主 + TUI 辅双模。数据采集双后端降低准入门槛。

## 已敲定的点

### 数据源
- **双后端**：CC Switch 适配器（直读 `cc-switch.db`）+ 原生文件扫描器（读各工具 session 日志）
- 两个 adapter 输出同一种 `UnifiedMessage`，上层全复用
- CC Switch 用户零成本接入，无 CC Switch 也能用

### 归一化 Token 模型
- `TokenBreakdown { input, output, cache_read, cache_write, reasoning }`
- 抹平 Anthropic 协议（cache_read 独立）与 OpenAI 协议（cached_tokens 含在 input 中）的差异
- 缓存命中率 = cache_read / (input + cache_read)

### 覆盖范围（MVP 全部）
- **工具**：Claude Code / Codex（CC Switch 覆盖）、Kimi Code / Gemini CLI / Pi / OpenClaw / Hermess（原生扫描覆盖）
- **指标**：input tokens / output tokens / cache read tokens / cache write tokens / total tokens / 缓存命中率 / 请求日志 / 模型统计 / 模型定价

### 界面
- **Web 主界面**：仪表盘、历史趋势、模型对比、请求日志详情
- **TUI 轻量辅助**：`usage-monitor tui` 子命令，实时概览、今日用量
- **不做**：原生桌面 GUI

### 定价
- 参考 Tokscale 三层查找：LiteLLM + OpenRouter + 手写 fallback
- 复用 CC Switch 已有的 `model_pricing` 表

### 技术约束
- 本地优先，无云依赖
- Windows 一等支持
- 参考 Tokscale 但有自己的代码风格和 UI 设计

## 遗留问题 & 下一步

- 技术栈最终选型（Rust 像 Tokscale 一样利索，还是 Node.js 像 TokenTracker 一样好分发）
- Web 前端用什么（React + ECharts？还是轻量方案？）
- TUI 用什么库（Rust 就 ratatui，Node.js 就 blessed/ink）
- 自己的 SQLite 存储 schema 设计
- CC Switch DB schema 是否稳定（版本升级会不会变）
- 各工具 session 日志路径和格式的差异细节（Tokscale 已验证 25+，我们覆盖 7 个）
