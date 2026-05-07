---
doc_type: roadmap
slug: usage-monitor
status: active
created: 2026-05-07
last_reviewed: 2026-05-07
tags: [token-usage, ai-coding-tools, monitoring, cost-tracking]
related_requirements: [usage-monitor]
related_architecture: []
---

# UsageMonitor — AI 编码工具 Token 用量监控

## 1. 背景

使用多个 AI 编码工具（Claude Code、Codex、Kimi Code、Gemini CLI、Pi、OpenClaw、Hermess）的开发者，token 用量散落在各家 session 日志里——格式不同、协议不同。CC Switch 用户有代理层数据但覆盖不全，非 CC Switch 用户完全看不见。

UsageMonitor 是一个本地工具，双数据源（CC Switch DB + 原生 session 扫描）+ 协议归一化 + Web 仪表盘 + 轻量 TUI，统一展示所有工具的 token 消耗和花费。架构参考 Tokscale 的归一化消息模型。

## 2. 范围与明确不做

### 本 roadmap 覆盖

- 统一数据模型（`UnifiedMessage` + `TokenBreakdown`），抹平 Anthropic/OpenAI 协议差异
- 双数据源：CC Switch 适配器 + 原生文件扫描器
- 7 个工具的 session 解析：Claude Code、Codex、Kimi Code、Gemini CLI、Pi、OpenClaw、Hermess
- 聚合引擎：按天 / 按模型 / 按 session 的用量统计
- 定价引擎：LiteLLM + OpenRouter + 手写 fallback，CC Switch 已有定价数据直接复用
- 本地 SQLite 存储，支持历史趋势查询
- Web 仪表盘：概览、模型对比、请求日志、趋势图
- 轻量 TUI：终端内快速查看今日用量

### 明确不做

- 代理拦截（那是 CC Switch 的职责）
- 团队 / 多人共享额度管理
- 实时告警、预算阻断
- 云端同步 / 账号系统
- 原生桌面 GUI（菜单栏 app 等）
- 超过 7 个工具的支持（MVP 后按需扩展）

## 3. 模块拆分（概设）

```
UsageMonitor
├── core-engine       数据模型 + 扫描框架 + 聚合逻辑（纯库，零 UI）
├── cc-switch-adapter 读取 CC Switch SQLite DB，产出 UnifiedMessage 流
├── parsers           各工具 session 文件解析器集合
├── storage           本地 SQLite 存储层（写入 + 查询 API）
├── pricing           模型定价引擎（LiteLLM + OpenRouter + 手写 fallback）
├── web-dashboard     Web 仪表盘（HTTP API + 静态前端）
├── tui               终端 UI（类 htop）
└── cli               CLI 入口（子命令：serve / tui / scan）
```

### core-engine · 核心引擎
- **职责**：定义 `UnifiedMessage`、`TokenBreakdown` 等共享类型；提供协议归一化工具（Anthropic cache ↔ OpenAI cached 口径转换、缓存命中率计算）；文件扫描框架（Walker + 模式匹配，参考 Tokscale scanner.rs）；聚合逻辑（按日 / 按模型 / 按 session 聚合 UnifiedMessage 流）。纯库，带单元测试。
- **承载的子 feature**：core-engine
- **触碰的现有代码**：全新

### cc-switch-adapter · CC Switch 适配器
- **职责**：直读 `~/.cc-switch/cc-switch.db` 的 `proxy_request_logs` 表和 `model_pricing` 表，转换为 `UnifiedMessage` 流。实现 `DataSource` trait。零配置，检测到 DB 文件存在即启用，不存在则 `enabled() = false`。
- **承载的子 feature**：cc-switch-adapter
- **触碰的现有代码**：全新（外部依赖 CC Switch DB schema）

### parsers · 原生解析器集合
- **职责**：每个工具一个 parser 模块，读原生 session 文件格式 → `UnifiedMessage`。处理各工具的边界条件（streaming 去重、增量 delta 计算、cache-inclusive 修正等，参考 Tokscale sessions/ 下各 parser 的处理方式）
- **承载的子 feature**：parsers-native
- **触碰的现有代码**：全新

### storage · 存储层
- **职责**：SQLite schema 定义（messages 表 + daily_rollups 表 + model_pricing 表）；写入 API（批量插入 UnifiedMessage + upsert 日聚合）；查询 API（按时间范围 / 模型 / 客户端 / session 查询）
- **承载的子 feature**：storage-layer
- **触碰的现有代码**：全新

### pricing · 定价引擎
- **职责**：从 LiteLLM 和 OpenRouter 拉取模型定价数据（磁盘缓存）；提供 `lookup(model_id) -> Option<ModelPricing>` 查询接口；复用 CC Switch 的 `model_pricing` 表作为额外数据源；手写 fallback 覆盖已知模型但上游缺失的情况（参考 Tokscale 的 Cursor overrides 机制）
- **承载的子 feature**：pricing-engine
- **触碰的现有代码**：全新

### web-dashboard · Web 仪表盘
- **职责**：HTTP API + 内嵌前端静态资源。API 提供 summary / daily / models / sessions / requests / scan 端点。前端展示概览卡片 + 模型用量排行 + 请求日志表格 + 趋势图
- **承载的子 feature**：web-dashboard
- **触碰的现有代码**：全新

### tui · 终端 UI
- **职责**：`usage-monitor tui` 子命令；终端内实时概览（今日 tokens / 花费 / 缓存命中率）、模型用量排行；键盘导航，定时自动刷新。参考 Tokscale TUI 的 tab 结构和 ratatui 实践
- **承载的子 feature**：tui
- **触碰的现有代码**：全新

### cli · CLI 入口
- **职责**：解析子命令（`serve` 起 Web 服务 / `tui` 进终端 / `scan` 手动触发扫描）；统一初始化（加载配置、触发扫描、启动对应模式）
- **承载的子 feature**：cli-entry
- **触碰的现有代码**：全新

## 4. 模块间接口契约 / 共享协议（架构层详设）

### 4.1 UnifiedMessage & TokenBreakdown（共享数据结构）

所有模块通过 `UnifiedMessage` 交换数据。这是全系统的"通用货币"。

```
struct UnifiedMessage {
    client: String,          // "claude" | "codex" | "kimi" | "gemini" | "pi" | "openclaw" | "hermes"
    model_id: String,        // "deepseek-v4-pro" | "kimi-for-coding" | "gpt-5.4" | ...
    provider_id: String,     // "anthropic" | "openai" | "moonshot" | "google" | ...
    session_id: String,      // 各工具的 session UUID
    timestamp: i64,          // Unix 毫秒
    tokens: TokenBreakdown,
    cost: f64,               // USD，0 表示免费或未定价
    request_id: Option<String>,
    workspace: Option<String>,
    data_source: String,     // "cc-switch" | "native"
}

struct TokenBreakdown {
    input: i64,
    output: i64,
    cache_read: i64,         // Anthropic: cache_read_input_tokens; OpenAI: 从 cached_tokens 中分离
    cache_write: i64,        // Anthropic: cache_creation_input_tokens; OpenAI: 通常为 0
    reasoning: i64,          // 推理 token，无则 0
}
```

**约束**：
- `cache_read` 必须在各 parser 中归一化：OpenAI 协议的 `cached_tokens` 如果已包含在 `input` 中，必须从 `input` 扣除再填入 `cache_read`
- `cache_write` 表示本次请求新写入缓存、下次可被读取的 token 数
- `cost` 为 0 是合法的（免费模型或尚未定价），不等于"数据缺失"
- `data_source` 如实填写，用于前端展示数据来源
- **缓存命中率** = `cache_read / (input + cache_read)`，由 core-engine 提供计算函数，不在 parser 里算

### 4.2 DataSource trait（数据源抽象）

```
trait DataSource {
    fn name(&self) -> &str;
    fn enabled(&self) -> bool;    // 轻量检查：文件/DB 是否存在，不做解析
    fn collect(&self) -> Result<Vec<UnifiedMessage>, DataSourceError>;
}

enum DataSourceError {
    NotAvailable(String),
    ParseError(String),
    IoError(String),
}
```

**约束**：
- `enabled()` 必须轻量（stat 或 sqlite3_open 即返回）
- `collect()` 返回空 Vec 是正常情况（无新数据），不是错误
- 调用方按 `name()` 去重

### 4.3 Storage 接口

```sql
-- messages: 请求级明细
CREATE TABLE messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client TEXT NOT NULL,
    model_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    input_tokens INTEGER DEFAULT 0,
    output_tokens INTEGER DEFAULT 0,
    cache_read_tokens INTEGER DEFAULT 0,
    cache_write_tokens INTEGER DEFAULT 0,
    reasoning_tokens INTEGER DEFAULT 0,
    cost_usd REAL DEFAULT 0,
    request_id TEXT,
    workspace TEXT,
    data_source TEXT NOT NULL,
    UNIQUE(request_id, client, timestamp)
);

-- daily_rollups: 日聚合
CREATE TABLE daily_rollups (
    date TEXT NOT NULL,
    client TEXT NOT NULL,
    model_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    request_count INTEGER DEFAULT 0,
    input_tokens INTEGER DEFAULT 0,
    output_tokens INTEGER DEFAULT 0,
    cache_read_tokens INTEGER DEFAULT 0,
    cache_write_tokens INTEGER DEFAULT 0,
    cost_usd REAL DEFAULT 0,
    PRIMARY KEY (date, client, model_id, provider_id)
);

-- model_pricing: 本地定价缓存
CREATE TABLE model_pricing (
    model_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    input_cost_per_million REAL NOT NULL,
    output_cost_per_million REAL NOT NULL,
    cache_read_cost_per_million REAL DEFAULT 0,
    source TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
```

查询接口：

```
fn query_summary(range: TimeRange) -> Result<Summary, StorageError>;
fn query_daily(range: TimeRange) -> Result<Vec<DailyStats>, StorageError>;
fn query_models(range: TimeRange) -> Result<Vec<ModelStats>, StorageError>;
fn query_sessions(range: TimeRange) -> Result<Vec<SessionSummary>, StorageError>;
fn query_session_detail(session_id: &str) -> Result<Vec<RequestLog>, StorageError>;
fn insert_messages(messages: &[UnifiedMessage]) -> Result<usize, StorageError>;
fn upsert_daily_rollup(date: &str) -> Result<(), StorageError>;
```

### 4.4 Pricing 接口

```
fn lookup(model_id: &str, provider_id: Option<&str>) -> Option<ModelPricing>;

struct ModelPricing {
    input_cost_per_token: f64,
    output_cost_per_token: f64,
    cache_read_cost_per_token: Option<f64>,
    source: String,     // "LiteLLM" | "OpenRouter" | "CC-Switch" | "manual"
}
```

**查找优先级**：LiteLLM 精确匹配 → OpenRouter 匹配 → CC Switch model_pricing 表 → 手写 fallback

**约束**：
- 定价数据启动时异步拉取 + 磁盘缓存，不阻塞首次展示
- 拉取失败不影响其他功能（降级显示 token 用量不显示价格）
- CC Switch 的 `model_pricing` 表数据直接作为备选源，不重复拉取

### 4.5 Web API

```
GET  /api/health                  → { status: "ok", last_scan: i64 }
GET  /api/summary?days=7          → { total_tokens, total_cost, cache_hit_rate, daily: [...] }
GET  /api/daily?from=&to=         → DailyBreakdown[]
GET  /api/models?from=&to=        → ModelStats[]
GET  /api/sessions?from=&to=      → SessionSummary[]
GET  /api/sessions/:id            → SessionDetail { messages: RequestLog[] }
POST /api/scan                    → { status: "started" }
GET  /api/scan/status             → { scanning: bool, progress: { found, parsed } }
```

**约束**：
- 所有 GET 端点只读 storage，不触发扫描
- POST /api/scan 异步执行，立即返回
- 前端通过轮询 /api/scan/status 获取进度

### 4.6 TUI 数据接口

TUI 通过和 Web 相同的 storage 查询接口获取数据。自动刷新通过定时轮询 storage 实现。不直接调 core-engine。

## 5. 子 feature 清单

1. **core-engine** — 定义 UnifiedMessage、TokenBreakdown、DataSource trait、扫描框架、聚合逻辑。纯库 + 单元测试。
   - 所属模块：core-engine
   - 依赖：无
   - 状态：done
   - 对应 feature：2026-05-07-core-engine

2. **cli-entry** — CLI 入口框架：子命令解析（serve / tui / scan）、配置加载、统一初始化。
   - 所属模块：cli
   - 依赖：无（先搭壳，core-engine 后续注入）
   - 状态：done
   - 对应 feature：2026-05-08-cli-entry

3. **parsers-native** — 7 个工具的 session 文件解析器：Claude Code、Codex、Kimi Code、Gemini CLI、Pi、OpenClaw、Hermess。每个 parser 输入文件路径 → 输出 `Vec<UnifiedMessage>`。参考 Tokscale sessions/ 下各 parser 的处理方式，处理 streaming 去重、增量 delta、cache-inclusive 修正等边界条件。
   - 所属模块：parsers
   - 依赖：core-engine
   - 状态：planned

4. **cc-switch-adapter** — 读 CC Switch DB `proxy_request_logs` 表，转换为 UnifiedMessage。实现 DataSource trait。CC Switch 用户零配置获得 Claude Code / Codex 数据。
   - 所属模块：cc-switch-adapter
   - 依赖：core-engine
   - 状态：planned

5. **storage-layer** — SQLite schema 建表 + 写入 API + 查询 API。支持批量插入消息、日聚合 upsert、多维度查询。
   - 所属模块：storage
   - 依赖：core-engine
   - 状态：planned

6. **pricing-engine** — 模型定价数据拉取（LiteLLM + OpenRouter）+ 磁盘缓存 + 定价查询接口 + 手写 fallback。复用 CC Switch model_pricing 表。
   - 所属模块：pricing
   - 依赖：core-engine
   - 状态：planned

7. **web-dashboard** — HTTP API + 内嵌前端静态资源。首页概览卡片、模型用量排行柱状图、请求日志表格、每日趋势折线图。`usage-monitor serve` 启动。
   - 所属模块：web-dashboard
   - 依赖：core-engine、storage-layer、pricing-engine、parsers-native、cc-switch-adapter
   - 状态：planned

8. **tui** — `usage-monitor tui` 子命令。终端概览（今日 tokens / 花费 / 缓存命中率）、模型用量排行、键盘导航、定时刷新。
   - 所属模块：tui
   - 依赖：core-engine、storage-layer、pricing-engine
   - 状态：planned

**最小闭环**：第 1–2 条（core-engine + cli-entry + parsers-native 中的 Claude Code parser 先行）做完后，用户运行 `usage-monitor scan` 能扫描 Claude Code session 文件并在终端看到 token 用量摘要。

## 6. 排期思路

按"数据基础 → 采集 → 存储 → 定价 → 展示"推进：

- **第一阶段（#1–#3）**：core-engine + cli-entry + parsers-native（Claude Code 先行），建立数据模型 + 能采集数据 + 终端看结果。最小闭环。
- **第二阶段（#4–#5）**：cc-switch-adapter + storage-layer，补齐 CC Switch 数据源 + 数据落盘，为 Web/TUI 提供查询基础。
- **第三阶段（#6）**：pricing-engine，成本可视化的前提。
- **第四阶段（#7–#8）**：web-dashboard + tui，两条展示线可并行，Web 优先。

技术栈建议 Rust（参考 Tokscale），理由：单二进制分发、axum HTTP 生态成熟、ratatui TUI 有 Tokscale 实践可参考。前端方案在 web-dashboard 的 feature-design 阶段确定。

## 7. 观察项

- CC Switch DB schema 升级时 `proxy_request_logs` 表结构是否会变——需关注版本兼容
- 各工具 session 日志格式可能随版本变化——每个 parser 需要容错设计（malformed line 跳过 + 告警，不静默丢数据）
- 与 Tokscale 的 parser 实现有天然重叠——可以参照但不能照搬（Tokscale 是 MIT 协议，可参考架构思路）
