---
doc_type: requirement
slug: usage-monitor
pitch: 一个本地工具，统一看清你所有 AI 编码助手花了多少 token、多少钱
status: current
last_reviewed: 2026-05-08
implemented_by: []
tags: [token-usage, ai-coding-tools, monitoring, cost-tracking]
---

# 统一查看所有 AI 编码工具的 token 用量和花费

## 用户故事

- 作为一个同时用 Claude Code、Codex、Kimi Code 写代码的人，我想在一个地方看到所有工具的 token 消耗，而不是逐个翻各自的 session 日志。
- 作为一个通过 CC Switch 切换多家模型的人，我想知道 deepseek-v4-pro 和 gpt-5.4 各花了多少钱，而不是月底收到账单才吓一跳。
- 作为一个在意缓存命中率的人，我想看到每次对话里 cache 省了多少 token，而不是对着 Anthropic 和 OpenAI 两种协议自己手算。
- 作为一个控制月度 AI 预算的人，我想看到每天的用量趋势和预估月花费，而不是花完才知道超了。

## 为什么需要

现在用 AI 编码工具的人越来越多，但用量数据散落在各家工具的日志文件里——格式不同、协议不同、连 token 的统计口径都不一样。有的工具（比如 CC Switch）替 Claude Code / Codex 做了代理层的用量记录，但 Kimi Code、Gemini CLI 这些直连工具完全看不见。结果是大多数人不知道自己花了多少 token、多少钱、缓存到底省没省——要么不关心，要么想关心也没办法。

## 怎么解决

在本地跑一个工具，自动从 CC Switch 数据库（有的话）和各工具的 session 日志里收数据，把 Anthropic 和 OpenAI 两种协议的 token 拆解统一成一种展示方式。架构上参考 Tokscale 的归一化消息模型——每工具一个独立 parser，输出统一的 token 结构，上层聚合和展示完全复用。打开浏览器就是一个仪表盘，终端里也能快速扫一眼。

## 变更日志

- 2026-05-08：core-engine 实现完成，能力从愿景进入当前——TokenBreakdown、UnifiedMessage、DataSource trait、扫描框架、聚合逻辑已就绪。status 从 draft 升级为 current。

## 边界

- 它不是代理，不拦截、不修改任何工具的 API 请求。
- 它不管团队配额、权限控制、多人共享额度——只做个人用量监控。
- 没有 CC Switch 也能用（走直接读 session 日志），但 CC Switch 覆盖的工具数据会更准。
- 不做实时告警、预算阻断——只看历史，不干预。
- 不用联网——定价数据本地缓存，用量数据不出本机。
