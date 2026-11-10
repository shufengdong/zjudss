//! DataFrame catalog 合同、schema 注册表与上传表类型推断。
//!
//! catalog 定义逻辑形状和受控转换；物理 Parquet 读取与业务发布由各自边界负责。

mod inference;
mod schema;

pub use inference::{CandidateScore, InferenceEvidence, infer_table, infer_tables};
pub use schema::{
    CardCondition, CardDefault, CardEnum, CardRange, CardUnit, CatalogError, ColumnPolicy,
    ColumnPolicyMode, DataFrameContractKind, DataFrameSchemaContract, DataTypeContract,
    Discriminator, FrameSchema, FrameShape, RowCountConstraint, SchemaCatalog, SchemaFieldContract,
    SchemaRegistry, SchemaRule, SchemaVariant, StructCardContract, UnitContract,
    UnknownColumnPolicy, UploadConversion,
};
