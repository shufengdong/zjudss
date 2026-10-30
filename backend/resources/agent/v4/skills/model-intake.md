# 模型输入整理

## 何时使用
用户询问已上传的 DSS/bundle/CSV/Excel/Parquet、草稿表替换或参数修改时使用。单个 RSS 是脚本，不是模型输入。

## 执行步骤
1. 调用 `inspect_workspace` 读取 active/draft 与模型导入状态，只按 import 产出的 DataFrame schema 判断标准表。
2. 多个候选或整表覆盖关系不明确时，列出候选并让用户用导入确认按钮选择。
3. 修改前调用 `read_model_dataframe`，用等值条件查询目标行和实际旧值；只有 `matchCount == 1` 且用户意图明确时才能继续。
4. 通过 `modify_model` 写入可变草稿，携带读取时的旧值与当前 draft revision；冲突后重新检查工作区并重新读取目标行。
5. 导入完成或普通修改不会自动发布。只有用户点击“生成版本”，或当前自然语言消息明确要求发布/生成当前模型版本时，才发布不可变版本并自动激活。
6. 自然语言发布必须先调用 `inspect_workspace`，再将其返回的精确 draft id/revision 传给 `publish_model_version`。`draft_revision_conflict` 后重新检查工作区，不得自动发布新 revision。
7. 同一消息还包含研究意图时，发布成功后可以继续加载 `study-planning`；脚本生成和执行仍必须等待用户在研究配置卡中点击“确认并生成脚本”。

## 读取失败恢复
- `invalid_model_query` 或 `model_query_output_too_large`：减少投影列和等值条件后重试。
- `draft_revision_conflict`：重新调用 `inspect_workspace`，再读取目标行。
- `dataframe_read_failed`：停止修改并报告读取失败，不得继续调用 `modify_model`。

## 禁止推断
不得用文件名、工作表名或上传先后覆盖已有输入；不得接受服务器路径。

## 验收
草稿清单含标准表名、schema、行数、内容 hash 和诊断；只有 `publish_model_version` 成功后才能宣称已经生成并激活版本。
