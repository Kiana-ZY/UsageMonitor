# UsageMonitor 架构总入口

> 状态：初始化中
> 最后更新：2026-05-08

## 1. 项目简介

用量监控桌面应用，本地 Web + TUI 双模，统一展示所有 AI 编码工具的 token 消耗和花费。

## 2. 核心概念 / 术语表

| 术语 | 定义 |
|---|---|
| `TokenBreakdown` | 归一化 token 拆解：input / output / cache_read / cache_write / reasoning |
| `UnifiedMessage` | 一次 AI 请求的归一化记录，全系统"通用货币" |
| `DataSource` | 数据源 trait，定义 `enabled()` + `collect()` |
| cache_read | 从缓存读取的 input token 数（Anthropic 语义） |
| cache_write | 本次写入缓存的 token 数 |

## 3. 子系统 / 模块索引

```
UsageMonitor
├── core-engine       数据模型 + 扫描框架 + 聚合逻辑（纯库）  ← 已实现
├── cc-switch-adapter CC Switch DB 适配器                      ← 已实现
├── parsers           各工具 session 解析器（7 个 parser）            ← 已实现
├── storage           本地 SQLite 存储层                        ← 已实现
├── pricing           模型定价引擎（CC Switch + 手写 fallback）   ← 已实现
├── web-dashboard     Web 仪表盘（axum + Chart.js）                ← 已实现
├── tui               终端 UI（Overview + Models）               ← 已实现
└── cli               CLI 入口（serve / tui / scan）              ← 已实现
```

### core-engine（已实现）

- **职责**：定义 `TokenBreakdown`、`UnifiedMessage`、`DataSource` trait、协议归一化工具、文件扫描框架、聚合逻辑。纯库，零运行时依赖。
- **位置**：`crates/usage-monitor-core/`
- **接口**：见 Roadmap 4.1–4.2 节接口契约

## 4. 关键架构决定

- 归一化以 Anthropic cache 语义为基准（cache_read/write 独立于 input）
- DataSource trait 抽象所有数据源，轻量 `enabled()` 检查
- 扫描框架用 walkdir + 模式匹配
- 聚合层纯内存 Map-Reduce，不依赖 SQL

## 5. 已知约束 / 硬边界

- 全正约束：聚合输出各字段 ≥ 0
- 幂等性：同一输入聚合两次输出一致
- 空输入容忍：空 Vec 不报错
- 日期转换使用本地时区
- 纯同步库，不依赖 tokio
- 不做代理拦截、不做团队管理、不做实时告警
