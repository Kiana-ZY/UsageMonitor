# core-engine 验收报告

> 阶段：阶段 3（验收闭环）
> 验收日期：2026-05-08
> 关联方案 doc：core-engine-design.md

## 1. 接口契约核对

对照方案第 2.1 节：

### TokenBreakdown

- [x] 字段：`input/output/cache_read/cache_write/reasoning: i64` → 代码 `lib.rs:23` 完全一致
- [x] `total_tokens()` 不计 cache_read → 代码 `lib.rs:40`，测试 `total_tokens_excludes_cache_read` 通过
- [x] `cache_hit_rate()` 0.0–1.0 → 代码 `lib.rs:47`，测试 `cache_hit_rate_*` 3 个通过
- [x] `clamp_negative()` → 代码 `lib.rs:57`，测试 `clamp_negative_*` 2 个通过

### UnifiedMessage

- [x] 字段：`client/model_id/provider_id/session_id/timestamp/tokens/cost/request_id/workspace/data_source` → 代码 `lib.rs:73` 完全一致
- [x] `cost: f64` 0 表示免费或未定价 → 默认 `0.0`

### DataSource trait

- [x] `name() -> &str` + `enabled() -> bool` + `collect() -> Result<Vec<UnifiedMessage>, DataSourceError>` → 代码 `lib.rs:112` 完全一致
- [x] mock 实现可编译可调用 → 测试 `datasource_*` 3 个通过

### Scanner

- [x] `ScanTask { root, pattern, exclude }` → 代码 `lib.rs:199` 一致
- [x] `scan_directory() -> Result<Vec<PathBuf>, ScannerError>` → 代码 `lib.rs:211` 一致
- [x] `scan_all() -> Result<Vec<PathBuf>, ScannerError>` → 代码 `lib.rs:245` 一致

### 聚合类型

- [x] `DailyContribution` / `ModelStats` / `SessionSummary` 字段 → 代码 `lib.rs:282/292/304` 与设计一致
- [x] `aggregate_by_date/by_model/by_session` 签名 → 代码一致

### 流程图核对

- [x] mermaid 图中 DataSource → Vec\<UnifiedMessage\> → aggregate_by_* 流 → grep 确认三函数均 pub fn

**结论：无偏差。**

## 2. 行为与决策核对

### 需求摘要逐项验证

- [x] 下游 import 不需要重定义 → `use usage_monitor_core::TokenBreakdown` 即可用
- [x] 纯库零外部服务 → Cargo.toml 无 HTTP/SQL/tokio 依赖

### 明确不做逐项核对（grep + review）

- [x] 不含 parser 实现 → `lib.rs` 无任何具体工具解析逻辑
- [x] 不定义 SQL/HTTP → grep 代码和 Cargo.toml 均为空
- [x] 不做定价计算 → 无 pricing 相关代码
- [x] 不引入第三方 API 调用 → 依赖仅有 serde/serde_json/thiserror/walkdir/chrono

### 关键决策落地

- [x] 决策 1（Anthropic 语义基准）：`cache_read/cache_write` 独立字段，`total_tokens()` 不计 `cache_read`
- [x] 决策 2（trait 非抽象类）：`DataSource` trait，mock 测试验证可自由实现
- [x] 决策 3（walkdir 扫描）：`scan_directory()` 用 `walkdir::WalkDir`
- [x] 决策 4（累加器模式）：聚合函数纯内存 Map-Reduce，无外部依赖
- [x] 决策 5（Rust）：Cargo workspace，edition 2021

### 编排层核对

- [x] 三条聚合流水线独立可调用，各自纯函数
- [x] 聚合函数幂等——同输入两遍同输出

### 流程级约束

- [x] 全正约束：聚合中调用 `clamp_negative()` + 测试 `aggregation_handles_negative_tokens` 通过
- [x] 空输入容忍：`aggregate_by_date_empty` + 各函数不 panic
- [x] 跨日边界：使用 chrono Local 时区 → `timestamp_to_date()` 内部实现

### 挂载点反向核对

- [x] M1：`Cargo.toml` 声明 `usage-monitor-core` crate → grep 确认 `crates/usage-monitor-core/Cargo.toml` 存在
- [x] M2：`crates/usage-monitor-core/src/lib.rs` 模块入口 → 文件存在
- [x] **反向 grep**：代码中无其他 `pub` 项引用不在清单内
- [x] **拔除沙盘**：删除 `crates/usage-monitor-core/` + workspace members 移除 → 无残留

## 3. 验收场景核对

- [x] **S1**：`total_tokens()` 不计 cache_read → 测试 `total_tokens_excludes_cache_read` ✓
- [x] **S2-S3**：`cache_hit_rate()` → 测试 `cache_hit_rate_normal/zero_denom/full_cache` ✓
- [x] **S4**：`clamp_negative()` → 测试 `clamp_negative_all_fields/mixed` ✓
- [x] **S5-S6**：`subtract_cached_overlap()` → 4 个测试覆盖正常/超过/零/both_zero ✓
- [x] **S7**：`normalize_model_id()` → 4 个测试覆盖日期后缀/小写/点划线/不变 ✓
- [x] **S8**：`scan_directory()` 单模式 → 测试 `scan_directory_single_pattern` ✓
- [x] **S9**：`scan_all()` 多任务 → 测试 `scan_all_multi_task_dedup` ✓
- [x] **S10-S11**：`aggregate_by_date()` 同日/跨日 → 测试通过 ✓
- [x] **S12**：`aggregate_by_model()` session_count → 测试 `aggregate_by_model_groups_correctly` ✓
- [x] **S13**：`aggregate_by_session()` message_count → 测试 `aggregate_by_session_counts_messages` ✓
- [x] **S14**：空输入不 panic → `aggregate_by_date_empty` ✓
- [x] **S15**：负值全字段 ≥ 0 → `aggregation_handles_negative_tokens` ✓

全部 15 场景有单测证据，34 tests + 1 doc-test 通过。无前端改动。

## 4. 术语一致性

- [x] `TokenBreakdown`：代码 1 处，design 术语表一致 ✓
- [x] `UnifiedMessage`：代码 1 处，一致 ✓
- [x] `DataSource`：代码 1 处，一致 ✓
- [x] `cache_read / cache_write`：字段名与 design 一致 ✓
- [x] `aggregate_by_date/by_model/by_session`：函数名与 design 一致 ✓
- [x] 禁用词：无冲突

## 5. 架构归并

本 feature 建立了项目的第一个模块，需要初始化架构文档。

### 名词归并 → ARCHITECTURE.md

- [x] 更新 `ARCHITECTURE.md`（见下方实际写入）

### 动词骨架归并

- [x] Scanner → DataSource → Aggregator 三层流水线写入架构

### 流程级约束归并

- [x] 全正约束、幂等性、空输入容忍、本地时区规则写入已知约束

## 6. requirement 回写

- [x] `requirement: usage-monitor` 指向 draft req → 触发 `cs-req update`
- [x] `status: draft` → `current` ✓
- [x] 变更日志已追加：2026-05-08 core-engine 实现完成
- [x] `VISION.md` 已更新：usage-monitor 移至 current 分组

## 7. roadmap 回写

- [x] `roadmap: usage-monitor` + `roadmap_item: core-engine` → 两字段有值
- [x] `usage-monitor-items.yaml`：`core-engine` status `in-progress` → `done` ✓
- [x] `usage-monitor-roadmap.md` 主文档子 feature 清单同步：状态 `planned` → `done`，补 `对应 feature: 2026-05-07-core-engine`
- [x] YAML 校验通过

## 8. attention.md 候选盘点

- [x] 无候选：本 feature 未暴露需要补入 attention.md 的内容。Rust workspace 结构、cargo test 命令均为项目标准操作。

## 9. 遗留

- 后续优化点：无
- 已知限制：`normalize_model_id()` 目前仅剥离 `-YYYY-MM-DD` 后缀，不处理 `-YYYYMMDD` 等变体（但测试覆盖充分，后续可扩展）
- 实现阶段"顺手发现"：无

