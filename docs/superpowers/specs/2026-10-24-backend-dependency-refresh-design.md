# Backend Dependency Refresh Design

## Intent

升级后端直接依赖到当前稳定版本并删除不再需要的依赖，降低已知缺陷和维护陈旧版本的风险，同时保持 V3 的公共行为与硬边界不变。

## Scope and constraints

- 保留用户已将 package 从 `backend` 重命名为 `zjudss` 的未提交修改。
- 允许必要的依赖主版本升级，并只做由升级直接要求的兼容性修改。
- 只有在源码、测试及 feature target 均不需要某个直接依赖时才删除它；仅在测试中使用的依赖保留在 `dev-dependencies`。
- 不改变 REST/SSE 合同、数据库 schema、Agent 工具、tensoreval adapter 边界或 Parquet 结果链路。
- 不进行无关重构，不主动提高项目 MSRV；以仓库当前 Rust 工具链和 edition 2024 为准。

## Approaches

- 保守升级：仅刷新现有 semver 范围内的锁文件，风险最低，但会留下旧主版本。
- 稳健升级（采用）：升级直接依赖到当前稳定版本，修复必要兼容问题，并删除经验证未使用的依赖。
- 深度精简：额外裁剪 feature 和间接依赖，体积收益可能更高，但回归范围明显扩大。

## Acceptance

- `Cargo.toml` 中的直接依赖均有实际用途，并处于本次可验证的当前稳定版本。
- `Cargo.lock` 与 manifest 一致，且不存在无意引入的重复旧版本或默认 feature 扩张。
- `cargo fmt --all -- --check`、全 target 编译和 `cargo test --features e2e-test` 通过。
- 现有公共行为、稳定错误码和功能目录不发生变化，因此无需更新 API、数据库或前端设计文档。
