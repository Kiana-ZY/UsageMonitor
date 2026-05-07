---
doc_type: feature-design
feature: 2026-05-08-cc-switch-adapter
requirement: usage-monitor
roadmap: usage-monitor
roadmap_item: cc-switch-adapter
status: approved
summary: CC Switch SQLite DB 适配器——读 proxy_request_logs 表转 UnifiedMessage，实现 DataSource trait，零配置
tags: [cc-switch, adapter, data-source]
---

# cc-switch-adapter

## 0. 术语

| 术语 | 定义 |
|---|---|
| CC Switch | 用户本地运行的模型代理工具，已有 token 用量数据库 |
| `cc-switch.db` | CC Switch 的 SQLite 数据库，路径 `~/.cc-switch/cc-switch.db` |

## 1. 决策与约束

- **做什么**：新 crate `usage-monitor-cc-switch`，读 CC Switch DB `proxy_request_logs` 表 → `Vec<UnifiedMessage>`。实现 `DataSource` trait。DB 不存在 → `enabled() = false`。
- **明确不做**：不改写 CC Switch DB；不拉取定价（pricing-engine 负责）
- **成功标准**：`usage-monitor scan` 同时输出 CC Switch 数据和 native 数据
- **DB schema（已知）**：`proxy_request_logs(request_id, provider_id, app_type, model, input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens, total_cost_usd, session_id, created_at, data_source)`

## 2. 名词与编排

**现状**：无 CC Switch 集成。

**变化**：新增 crate `crates/usage-monitor-cc-switch/`，模块 `cc_switch.rs`。

### 挂载点
- `Cargo.toml` workspace + 依赖
- `src/main.rs` scan 命令新增 CC Switch 数据源
- `crates/usage-monitor-cc-switch/src/lib.rs`

### 推进
1. 创建 crate + DB 读取
2. 挂接 CLI scan
3. 校验收尾

## 3. 验收

| # | 场景 | 期望 |
|---|---|---|
| S1 | CC Switch DB 存在 | 读出数据，model 正确 |
| S2 | CC Switch DB 不存在 | enabled()=false，不报错 |
| S3 | CLI scan 同时显示 CC Switch + native |

## 4. 架构

parsers 模块加 adapter 子模块，ARCHITECTURE.md cc-switch-adapter → 已实现。
