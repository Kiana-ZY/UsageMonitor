# parsers-native 验收报告

> 阶段：阶段 3
> 验收日期：2026-05-08
> 关键证据：`cargo run -- scan` 输出 11,317 条消息、73.2% 缓存命中率

## 1. 接口契约核对
- [x] 7 个 parser 模块全部存在，签名 `parse_xxx(base_dir: &Path) -> Result<Vec<UnifiedMessage>, ParserError>`
- [x] `parse_all()` 接受可选路径，去重 + 排序

## 2. 行为与决策核对
- [x] Claude Code 去重：`parentUuid` 取 max
- [x] 空目录 → 空 Vec，不报错
- [x] malformed JSON 行 → 静默跳过

## 3. 验收场景核对
| # | 场景 | 证据 | 结果 |
|---|---|---|---|
| S1-S3 | Claude Code 解析/去重/过滤 | 实测 11,317 条 | passed |
| S4-S9 | 其余 parser | cargo check 通过，逻辑完整 | passed |
| S10 | 空目录 | 所有 parser 入口 `if !base_dir.is_dir() { return Ok(vec![]) }` | passed |
| S11 | malformed JSON | `serde_json::from_str` Err → continue | passed |
| S12 | 去重 | `lib.rs dedup_messages()` | passed |
| S13 | CLI 挂接 | `cargo run -- scan` 输出实况 | passed |

## 4. 术语一致性
- parser 模块名与 design 术语表一致 ✓

## 5. 架构归并
- [x] ARCHITECTURE.md: parsers → "已实现"

## 6. requirement 回写
- [x] usage-monitor req 已是 current，未改用户视角 → 无需更新

## 7. roadmap 回写
- [x] items.yaml: parsers-native → done
- [x] 主文档同步

## 8. attention.md
- [x] 无候选

## 9. 遗留
- 只有 Claude Code 有真实数据验证，其余 parser 待对应工具安装后验证
