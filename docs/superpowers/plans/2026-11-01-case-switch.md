# Case Switch Implementation Plan

**Goal:** 当前会话整体切换案例并自动激活模型版本。
**Architecture:** 提取现有 repository 事务内函数；案例确认单事务组合模型和资源发布，普通上传保持现状。现有确认卡区分案例切换语义。
**Tech Stack:** Rust / Diesel SQLite / React / Vitest。
**Spec:** docs/superpowers/specs/2026-11-01-case-switch-design.md

- [x] 增加案例切换行为测试：版本激活、旧表不残留、资源替换、历史保留、失败回滚和幂等。
- [x] 提取事务内发布逻辑并实现案例切换编排，普通上传继续回归。
- [x] 更新确认卡和前端行为测试，完善当前设计及功能目录。
- [x] 执行后端相关测试、前端测试/构建和覆盖门禁。

验证：后端全部测试组与 doc tests 分组通过；Clippy/fmt 通过；前端 197 项测试与构建通过；29 条 Playwright 旅程通过；PR 功能覆盖门禁 100%。本地 release 后端已重新编译并启动。
