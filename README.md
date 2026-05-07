# UsageMonitor

AI 编码工具 Token 用量监控——统一看清所有 AI 编码助手花了多少 token、多少钱。

**本地优先**，双数据源（CC Switch + 原生 session 日志），Web 仪表盘 + 终端 TUI。

```
cargo run -- serve    # Web 仪表盘 → http://localhost:4317
cargo run -- tui      # 终端 UI
cargo run -- scan     # 扫描全部数据源 + 输出统计
```

## 支持的工具

| 工具 | 数据源 |
|---|---|
| Claude Code | CC Switch DB / 原生 session JSONL |
| Codex CLI | CC Switch DB / 原生 session JSONL |
| Kimi Code | 原生 wire.jsonl |
| Gemini CLI | 原生 session JSON/JSONL |
| Pi | 原生 session JSONL |
| OpenClaw | 原生 session JSONL |
| Hermess | 原生 SQLite |

## 功能

- **协议归一化**：统一 Anthropic / OpenAI 两种协议的 token 拆解，缓存命中率跨协议一致
- **双数据源**：CC Switch 用户零配置接入；无 CC Switch 也能用原生扫描
- **Web 仪表盘**：概览卡片 + 每日趋势柱状图 + 模型排行 + Session 列表
- **终端 TUI**：Overview + Models 两个 Tab，纯键盘操作
- **本地 SQLite 存储**：历史趋势查询，数据不出本机
- **模型定价**：CC Switch 定价表 + 手写 fallback，自动计算花费

## 编译运行

```bash
# 要求 Rust 1.94+
cargo build --release

# Web 仪表盘
./target/release/usage-monitor serve

# 终端 TUI
./target/release/usage-monitor tui

# 手动扫描
./target/release/usage-monitor scan
```

## 架构

```
crates/
├── usage-monitor-core/       数据模型 + 聚合引擎
├── usage-monitor-parsers/    7 工具 session 解析器
├── usage-monitor-cc-switch/  CC Switch DB 适配器
├── usage-monitor-storage/    SQLite 存储层
├── usage-monitor-pricing/    模型定价引擎
├── usage-monitor-web/        axum Web 仪表盘
└── usage-monitor-tui/        ratatui 终端 UI
```

## 协议

MIT
