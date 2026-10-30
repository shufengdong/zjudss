use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::engine_resources::EngineResourceSnapshot;

#[derive(Debug, Clone)]
pub struct FrameSchema {
    pub kind: String,
    pub required: BTreeMap<String, String>,
    pub optional: BTreeMap<String, String>,
    pub stable_keys: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub struct SchemaCatalog {
    pub contract_version: String,
    pub schema_version: u32,
    pub frames: Vec<FrameSchema>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataFrameContractKind {
    Model,
    Struct,
    Data,
    Result,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowCountConstraint {
    Zero,
    One,
    ZeroOrOne,
    ZeroOrMore,
    OneOrMore,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameShape {
    Uniform,
    Discriminated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnPolicyMode {
    Closed,
    Open,
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownColumnPolicy {
    Forbidden,
    Ignored,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnPolicy {
    pub mode: ColumnPolicyMode,
    pub unknown_columns: UnknownColumnPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DataTypeContract {
    Primitive { name: String },
    Semantic { name: String },
    Constructor { name: String, element: Box<Self> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UnitContract(pub String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardDefault {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardUnit {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardRange {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum_inclusive: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum_inclusive: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardEnum {
    pub state: String,
    #[serde(default)]
    pub values: Vec<Value>,
    #[serde(default)]
    pub case_insensitive: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardCondition {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructCardContract {
    pub default: CardDefault,
    pub unit: CardUnit,
    pub range: CardRange,
    #[serde(rename = "enum")]
    pub enum_values: CardEnum,
    pub visible_when: CardCondition,
    pub enabled_when: CardCondition,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaFieldContract {
    pub name: String,
    pub dtype: DataTypeContract,
    pub required: bool,
    pub nullable: bool,
    pub unit: UnitContract,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card: Option<StructCardContract>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadConversion {
    NumericToU32Semantics,
    ScalarToF64List,
    FlatListToMatrix,
    ScalarToStringList,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "contract", rename_all = "snake_case")]
pub enum SchemaRule {
    Positive(Value),
    Nonnegative(Value),
    Range(Value),
    Compare(Value),
    ConditionalCompare(Value),
    ConditionalRequired(Value),
    ConditionalEnabled(Value),
    ConditionalIgnored(Value),
    ConditionalEmptyInput(Value),
    MutuallyExclusive(Value),
    MutuallyExclusiveGroups(Value),
    PairedPresence(Value),
    Length(Value),
    LengthEqual(Value),
    EqualLength(Value),
    Unique(Value),
    UniqueItems(Value),
    UniqueCsvItems(Value),
    Finite(Value),
    Requires(Value),
    Forbidden(Value),
    Enum(Value),
    StrictlyIncreasing(Value),
    PositiveContiguous(Value),
    ContiguousZeroBased(Value),
    ExactlyOneVariant(Value),
    CrossTableDenseMatrix(Value),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Discriminator {
    pub field: String,
    pub source: String,
    #[serde(default)]
    pub synthetic: bool,
    #[serde(default)]
    pub case_insensitive: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaVariant {
    pub id: String,
    pub when: Value,
    pub required: Vec<String>,
    pub optional: Vec<String>,
    pub enabled: Vec<String>,
    pub side_schemas: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_count: Option<RowCountConstraint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataFrameSchemaContract {
    pub schema_ref: String,
    pub kind: DataFrameContractKind,
    pub row_count: RowCountConstraint,
    pub shape: FrameShape,
    pub column_policy: ColumnPolicy,
    pub fields: Vec<SchemaFieldContract>,
    pub stable_keys: Vec<String>,
    pub sort_keys: Vec<String>,
    pub column_aliases: BTreeMap<String, String>,
    pub allowed_conversions: Vec<UploadConversion>,
    pub rules: Vec<SchemaRule>,
    pub discriminator: Option<Discriminator>,
    pub variants: Vec<SchemaVariant>,
    pub schema_hash: String,
}

#[derive(Debug, Clone)]
struct DtypeGrammar {
    primitives: BTreeMap<String, Value>,
    constructors: BTreeMap<String, Value>,
    semantics: BTreeMap<String, Value>,
    conversions: BTreeMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct SchemaRegistry {
    raw: Value,
    grammar: DtypeGrammar,
    resolved: BTreeMap<String, DataFrameSchemaContract>,
}

#[derive(Debug, Clone)]
pub struct CatalogError {
    code: &'static str,
    message: String,
}

impl CatalogError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CatalogError {}

impl SchemaRegistry {
    pub(crate) fn from_catalog(root: &Value) -> Result<Self, CatalogError> {
        let grammar = root
            .get("dtype_grammar")
            .and_then(Value::as_object)
            .ok_or_else(|| CatalogError::new("catalog_invalid", "缺少 dtype_grammar"))?;
        let map = |name: &str| -> Result<BTreeMap<String, Value>, CatalogError> {
            Ok(grammar
                .get(name)
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    CatalogError::new("catalog_invalid", format!("dtype_grammar 缺少 {name}"))
                })?
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect())
        };
        Ok(Self {
            raw: root.clone(),
            grammar: DtypeGrammar {
                primitives: map("primitives")?,
                constructors: map("constructors")?,
                semantics: map("semantic_types")?,
                conversions: map("upload_conversions")?,
            },
            resolved: BTreeMap::new(),
        })
    }

    pub fn resolve(&self, schema_ref: &str) -> Result<&DataFrameSchemaContract, CatalogError> {
        self.resolved.get(schema_ref).ok_or_else(|| {
            CatalogError::new(
                "catalog_schema_not_resolved",
                format!("schema 未由任何门面引用: {schema_ref}"),
            )
        })
    }

    pub(crate) fn resolve_for_facade(
        &mut self,
        schema_ref: &str,
    ) -> Result<DataFrameSchemaContract, CatalogError> {
        let mut visiting = Vec::new();
        self.resolve_inner(schema_ref, &mut visiting)
    }

    fn resolve_inner(
        &mut self,
        schema_ref: &str,
        visiting: &mut Vec<String>,
    ) -> Result<DataFrameSchemaContract, CatalogError> {
        if let Some(schema) = self.resolved.get(schema_ref) {
            return Ok(schema.clone());
        }
        if visiting.iter().any(|item| item == schema_ref) {
            return Err(CatalogError::new(
                "catalog_schema_cycle",
                format!(
                    "base_schema 循环: {} -> {schema_ref}",
                    visiting.join(" -> ")
                ),
            ));
        }
        let (section, name) = split_schema_ref(schema_ref)?;
        let definition = self
            .raw
            .get(section)
            .and_then(Value::as_object)
            .and_then(|entries| entries.get(name))
            .cloned()
            .ok_or_else(|| {
                CatalogError::new(
                    "catalog_schema_missing",
                    format!("缺少 schema: {schema_ref}"),
                )
            })?;
        visiting.push(schema_ref.to_string());
        let resolved = self.parse_definition(schema_ref, &definition, visiting);
        visiting.pop();
        let resolved = resolved?;
        self.resolved
            .insert(schema_ref.to_string(), resolved.clone());
        Ok(resolved)
    }

    fn parse_definition(
        &mut self,
        schema_ref: &str,
        definition: &Value,
        visiting: &mut Vec<String>,
    ) -> Result<DataFrameSchemaContract, CatalogError> {
        let definition = definition.as_object().ok_or_else(|| {
            CatalogError::new(
                "catalog_schema_invalid",
                format!("schema 不是对象: {schema_ref}"),
            )
        })?;
        let contract = object(definition.get("contract"), "contract", schema_ref)?;
        let kind = parse_kind(string(contract.get("kind"), "contract.kind", schema_ref)?)?;
        let row_count = parse_row_count(string(
            contract.get("rowCount"),
            "contract.rowCount",
            schema_ref,
        )?)?;
        let shape = parse_shape(string(contract.get("shape"), "contract.shape", schema_ref)?)?;
        let policy = object(
            contract.get("columnPolicy"),
            "contract.columnPolicy",
            schema_ref,
        )?;
        let column_policy = ColumnPolicy {
            mode: parse_column_mode(string(policy.get("mode"), "columnPolicy.mode", schema_ref)?)?,
            unknown_columns: parse_unknown_columns(string(
                policy.get("unknownColumns"),
                "columnPolicy.unknownColumns",
                schema_ref,
            )?)?,
        };
        let mut fields = array(contract.get("columns"), "contract.columns", schema_ref)?
            .iter()
            .map(|field| self.parse_field(field, &kind, schema_ref))
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(required) = definition
            .get("required_for_imported_network")
            .and_then(Value::as_object)
        {
            for name in required.keys() {
                let field = fields
                    .iter_mut()
                    .find(|field| field.name == *name)
                    .ok_or_else(|| {
                        CatalogError::new(
                            "catalog_schema_invalid",
                            format!(
                                "{schema_ref} required_for_imported_network 引用未知字段 {name}"
                            ),
                        )
                    })?;
                field.required = true;
                field.nullable = false;
            }
        }
        let mut rules = array(contract.get("rules"), "contract.rules", schema_ref)?
            .iter()
            .map(|rule| parse_rule(rule, schema_ref))
            .collect::<Result<Vec<_>, _>>()?;
        let mut allowed_conversions = array(
            contract.get("allowedConversions"),
            "contract.allowedConversions",
            schema_ref,
        )?
        .iter()
        .map(|conversion| self.parse_conversion(conversion, schema_ref))
        .collect::<Result<Vec<_>, _>>()?;
        let mut stable_keys = string_array(contract.get("stableKeys"), "stableKeys", schema_ref)?;
        let mut sort_keys = string_array(contract.get("sortKeys"), "sortKeys", schema_ref)?;
        let mut column_aliases =
            string_map(contract.get("columnAliases"), "columnAliases", schema_ref)?;
        let discriminator = contract
            .get("discriminator")
            .map(|value| parse_discriminator(value, schema_ref))
            .transpose()?;
        let variants = contract
            .get("variants")
            .map(|value| {
                array(Some(value), "variants", schema_ref)?
                    .iter()
                    .map(|variant| parse_variant(variant, schema_ref))
                    .collect::<Result<Vec<_>, CatalogError>>()
            })
            .transpose()?
            .unwrap_or_default();

        if let Some(base_ref) = definition.get("base_schema").and_then(Value::as_str) {
            let merge = object(contract.get("baseMerge"), "contract.baseMerge", schema_ref)?;
            let order = string(merge.get("order"), "baseMerge.order", schema_ref)?;
            if !matches!(order, "base_then_local" | "local_then_base")
                || string(merge.get("override"), "baseMerge.override", schema_ref)? != "forbidden"
            {
                return Err(CatalogError::new(
                    "catalog_schema_invalid",
                    format!("{schema_ref} 的 baseMerge 无效"),
                ));
            }
            let base = self.resolve_inner(base_ref, visiting)?;
            if base.kind != kind {
                return Err(CatalogError::new(
                    "catalog_schema_invalid",
                    format!("{schema_ref} 与 base_schema kind 不一致"),
                ));
            }
            fields = if order == "base_then_local" {
                let mut merged = base.fields.clone();
                merged.append(&mut fields);
                merged
            } else {
                fields.extend(base.fields.clone());
                fields
            };
            reject_duplicate_field_names(&fields, schema_ref)?;
            let mut base_rules = base.rules.clone();
            base_rules.append(&mut rules);
            rules = base_rules;
            let mut conversions = base.allowed_conversions.clone();
            conversions.append(&mut allowed_conversions);
            conversions.sort();
            conversions.dedup();
            allowed_conversions = conversions;
            if stable_keys.is_empty() {
                stable_keys = base.stable_keys;
            }
            if sort_keys.is_empty() {
                sort_keys = base.sort_keys;
            }
            for (alias, canonical) in base.column_aliases {
                if column_aliases.insert(alias.clone(), canonical).is_some() {
                    return Err(CatalogError::new(
                        "catalog_schema_invalid",
                        format!("{schema_ref} 重复 column alias: {alias}"),
                    ));
                }
            }
            // Shape selection belongs to the derived contract. Inherited schemas
            // share columns and executable rules, while a derived uniform frame
            // may intentionally flatten a discriminated base.
        } else if contract.get("baseMerge").is_some() {
            return Err(CatalogError::new(
                "catalog_schema_invalid",
                format!("{schema_ref} 没有 base_schema 却声明 baseMerge"),
            ));
        }

        let mut output = DataFrameSchemaContract {
            schema_ref: schema_ref.to_string(),
            kind,
            row_count,
            shape,
            column_policy,
            fields,
            stable_keys,
            sort_keys,
            column_aliases,
            allowed_conversions,
            rules,
            discriminator,
            variants,
            schema_hash: String::new(),
        };
        validate_contract(schema_ref, &output, contract.get("source"))?;
        validate_rule_references(schema_ref, &output)?;
        let field_names = output
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<BTreeSet<_>>();
        for variant in &output.variants {
            for field in variant
                .required
                .iter()
                .chain(&variant.optional)
                .chain(&variant.enabled)
            {
                if !field_names.contains(field.as_str()) {
                    return Err(CatalogError::new(
                        "catalog_schema_invalid",
                        format!("{schema_ref} variant {} 引用未知字段 {field}", variant.id),
                    ));
                }
            }
        }
        let mut side_schema_hashes = BTreeMap::new();
        for side_schema in output
            .variants
            .iter()
            .flat_map(|variant| variant.side_schemas.iter())
        {
            let side = self.resolve_inner(side_schema, visiting)?;
            side_schema_hashes.insert(side_schema.clone(), side.schema_hash);
        }
        let execution = json!({
            "contract": &output,
            "dtypeDefinitions": self.referenced_dtype_definitions(&output)?,
            "conversionDefinitions": self.referenced_conversion_definitions(&output)?,
            "sideSchemas": side_schema_hashes,
        });
        output.schema_hash = hash_json(&execution);
        Ok(output)
    }

    fn parse_field(
        &self,
        field: &Value,
        kind: &DataFrameContractKind,
        schema_ref: &str,
    ) -> Result<SchemaFieldContract, CatalogError> {
        let field = object(Some(field), "column", schema_ref)?;
        let name = string(field.get("name"), "column.name", schema_ref)?.to_string();
        let dtype = self.parse_dtype(string(field.get("dtype"), "column.dtype", schema_ref)?)?;
        let required = boolean(field.get("required"), "column.required", schema_ref)?;
        let nullable = boolean(field.get("nullable"), "column.nullable", schema_ref)?;
        let unit = string(field.get("unit"), "column.unit", schema_ref)?;
        let card = field
            .get("card")
            .map(|card| parse_card(card, schema_ref, &name))
            .transpose()?;
        if *kind == DataFrameContractKind::Struct && card.is_none() {
            return Err(CatalogError::new(
                "catalog_schema_invalid",
                format!("{schema_ref}.{name} 缺少 Struct card"),
            ));
        }
        if *kind != DataFrameContractKind::Struct && card.is_some() {
            return Err(CatalogError::new(
                "catalog_schema_invalid",
                format!("{schema_ref}.{name} 非 Struct 字段不能声明 card"),
            ));
        }
        Ok(SchemaFieldContract {
            name,
            dtype,
            required,
            nullable,
            unit: UnitContract(unit.to_string()),
            card,
        })
    }

    fn parse_dtype(&self, raw: &str) -> Result<DataTypeContract, CatalogError> {
        if self.grammar.primitives.contains_key(raw) {
            return Ok(DataTypeContract::Primitive {
                name: raw.to_string(),
            });
        }
        if self.grammar.semantics.contains_key(raw) {
            return Ok(DataTypeContract::Semantic {
                name: raw.to_string(),
            });
        }
        let Some(open) = raw.find('<') else {
            return Err(CatalogError::new(
                "catalog_dtype_invalid",
                format!("未知 dtype: {raw}"),
            ));
        };
        if !raw.ends_with('>') || open == 0 {
            return Err(CatalogError::new(
                "catalog_dtype_invalid",
                format!("非法 dtype: {raw}"),
            ));
        }
        let constructor = &raw[..open];
        if !self.grammar.constructors.contains_key(constructor) {
            return Err(CatalogError::new(
                "catalog_dtype_invalid",
                format!("未知 dtype constructor: {constructor}"),
            ));
        }
        let inner = &raw[open + 1..raw.len() - 1];
        if inner.is_empty() || inner.contains(',') {
            return Err(CatalogError::new(
                "catalog_dtype_invalid",
                format!("dtype constructor arity 无效: {raw}"),
            ));
        }
        Ok(DataTypeContract::Constructor {
            name: constructor.to_string(),
            element: Box::new(self.parse_dtype(inner)?),
        })
    }

    fn parse_conversion(
        &self,
        value: &Value,
        schema_ref: &str,
    ) -> Result<UploadConversion, CatalogError> {
        let name = value.as_str().ok_or_else(|| {
            CatalogError::new(
                "catalog_schema_invalid",
                format!("{schema_ref} conversion 不是字符串"),
            )
        })?;
        if !self.grammar.conversions.contains_key(name) {
            return Err(CatalogError::new(
                "catalog_schema_invalid",
                format!("{schema_ref} 使用未知 conversion: {name}"),
            ));
        }
        match name {
            "numeric_to_u32_semantics" => Ok(UploadConversion::NumericToU32Semantics),
            "scalar_to_f64_list" => Ok(UploadConversion::ScalarToF64List),
            "flat_list_to_matrix" => Ok(UploadConversion::FlatListToMatrix),
            "scalar_to_string_list" => Ok(UploadConversion::ScalarToStringList),
            _ => Err(CatalogError::new(
                "catalog_schema_invalid",
                format!("不支持 conversion: {name}"),
            )),
        }
    }

    fn referenced_dtype_definitions(
        &self,
        schema: &DataFrameSchemaContract,
    ) -> Result<BTreeMap<String, Value>, CatalogError> {
        let mut names = BTreeSet::new();
        for field in &schema.fields {
            collect_dtype_names(&field.dtype, &mut names);
        }
        names
            .into_iter()
            .map(|name| {
                let value = self
                    .grammar
                    .primitives
                    .get(&name)
                    .or_else(|| self.grammar.constructors.get(&name))
                    .or_else(|| self.grammar.semantics.get(&name))
                    .cloned()
                    .ok_or_else(|| {
                        CatalogError::new("catalog_dtype_invalid", format!("未知 dtype: {name}"))
                    })?;
                Ok((name, value))
            })
            .collect()
    }

    fn referenced_conversion_definitions(
        &self,
        schema: &DataFrameSchemaContract,
    ) -> Result<BTreeMap<String, Value>, CatalogError> {
        schema
            .allowed_conversions
            .iter()
            .map(|conversion| {
                let name = conversion_name(conversion);
                let value = self.grammar.conversions.get(name).cloned().ok_or_else(|| {
                    CatalogError::new("catalog_schema_invalid", format!("未知 conversion: {name}"))
                })?;
                Ok((name.to_string(), value))
            })
            .collect()
    }
}

impl SchemaCatalog {
    pub fn from_snapshot(
        snapshot: &EngineResourceSnapshot,
        contract_version: &str,
    ) -> Result<Self, CatalogError> {
        if snapshot.contract_version != contract_version {
            return Err(CatalogError::new(
                "engine_contract_mismatch",
                "请求合同与资源快照不一致",
            ));
        }
        Self::from_value(contract_version, &snapshot.catalog_json)
    }

    pub fn from_value(contract_version: &str, root: &Value) -> Result<Self, CatalogError> {
        let schema_version = root
            .get("schema_version")
            .and_then(Value::as_u64)
            .ok_or_else(|| CatalogError::new("catalog_invalid", "缺少 schema_version"))?
            as u32;
        let mut by_kind = BTreeMap::new();
        for section in ["frames", "analysis_inputs"] {
            if let Some(entries) = root.get(section).and_then(Value::as_object) {
                for (kind, definition) in entries {
                    if by_kind.contains_key(kind) {
                        continue;
                    }
                    if let Some(frame) = parse_frame(kind, definition) {
                        by_kind.insert(kind.clone(), frame);
                    }
                }
            }
        }
        if by_kind.is_empty() {
            return Err(CatalogError::new(
                "catalog_invalid",
                "catalog 没有可推断输入 schema",
            ));
        }
        Ok(Self {
            contract_version: contract_version.to_string(),
            schema_version,
            frames: by_kind.into_values().collect(),
        })
    }
}

fn parse_frame(kind: &str, value: &Value) -> Option<FrameSchema> {
    if let Some(columns) = value
        .get("contract")
        .and_then(|contract| contract.get("columns"))
        .and_then(Value::as_array)
    {
        let mut required = BTreeMap::new();
        let mut optional = BTreeMap::new();
        let required_for_imported_network = value
            .get("required_for_imported_network")
            .and_then(Value::as_object);
        for column in columns {
            let name = normalize_name(column.get("name")?.as_str()?);
            let dtype = column.get("dtype")?.as_str()?.to_string();
            if column.get("required")?.as_bool()?
                || required_for_imported_network.is_some_and(|fields| fields.contains_key(&name))
            {
                required.insert(name, dtype);
            } else {
                optional.insert(name, dtype);
            }
        }
        if required.is_empty() {
            return None;
        }
        let stable_keys = value
            .get("contract")
            .and_then(|contract| contract.get("stableKeys"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(normalize_name)
            .collect();
        return Some(FrameSchema {
            kind: kind.to_string(),
            required,
            optional,
            stable_keys,
        });
    }
    let required = [
        "required",
        "required_simple",
        "required_for_imported_network",
        "required_for_replay",
    ]
    .iter()
    .find_map(|name| value.get(*name).and_then(Value::as_object))
    .map(parse_columns)
    .unwrap_or_default();
    if required.is_empty() {
        return None;
    }
    let optional = value
        .get("optional")
        .and_then(Value::as_object)
        .map(parse_columns)
        .unwrap_or_default();
    let stable_keys = ["id", "name"]
        .into_iter()
        .filter(|candidate| required.contains_key(*candidate))
        .map(str::to_string)
        .collect();
    Some(FrameSchema {
        kind: kind.to_string(),
        required,
        optional,
        stable_keys,
    })
}

fn parse_columns(columns: &Map<String, Value>) -> BTreeMap<String, String> {
    columns
        .iter()
        .map(|(name, dtype)| {
            (
                normalize_name(name),
                dtype.as_str().unwrap_or("unknown").to_string(),
            )
        })
        .collect()
}

fn parse_kind(value: &str) -> Result<DataFrameContractKind, CatalogError> {
    match value {
        "model" => Ok(DataFrameContractKind::Model),
        "struct" => Ok(DataFrameContractKind::Struct),
        "data" => Ok(DataFrameContractKind::Data),
        "result" => Ok(DataFrameContractKind::Result),
        _ => Err(CatalogError::new(
            "catalog_schema_invalid",
            format!("未知 contract kind: {value}"),
        )),
    }
}

fn parse_row_count(value: &str) -> Result<RowCountConstraint, CatalogError> {
    match value {
        "zero" => Ok(RowCountConstraint::Zero),
        "one" => Ok(RowCountConstraint::One),
        "zero_or_one" => Ok(RowCountConstraint::ZeroOrOne),
        "zero_or_more" => Ok(RowCountConstraint::ZeroOrMore),
        "one_or_more" => Ok(RowCountConstraint::OneOrMore),
        _ => Err(CatalogError::new(
            "catalog_schema_invalid",
            format!("未知 rowCount: {value}"),
        )),
    }
}

fn parse_shape(value: &str) -> Result<FrameShape, CatalogError> {
    match value {
        "uniform" => Ok(FrameShape::Uniform),
        "discriminated" => Ok(FrameShape::Discriminated),
        _ => Err(CatalogError::new(
            "catalog_schema_invalid",
            format!("未知 shape: {value}"),
        )),
    }
}

fn parse_column_mode(value: &str) -> Result<ColumnPolicyMode, CatalogError> {
    match value {
        "closed" => Ok(ColumnPolicyMode::Closed),
        "open" => Ok(ColumnPolicyMode::Open),
        "dynamic" => Ok(ColumnPolicyMode::Dynamic),
        _ => Err(CatalogError::new(
            "catalog_schema_invalid",
            format!("未知 column policy mode: {value}"),
        )),
    }
}

fn parse_unknown_columns(value: &str) -> Result<UnknownColumnPolicy, CatalogError> {
    match value {
        "forbidden" => Ok(UnknownColumnPolicy::Forbidden),
        "ignored" => Ok(UnknownColumnPolicy::Ignored),
        _ => Err(CatalogError::new(
            "catalog_schema_invalid",
            format!("未知 unknownColumns: {value}"),
        )),
    }
}

fn parse_card(
    value: &Value,
    schema_ref: &str,
    field: &str,
) -> Result<StructCardContract, CatalogError> {
    let card = object(Some(value), "card", schema_ref)?;
    let context = format!("{schema_ref}.{field}.card");
    let default = object(card.get("default"), "card.default", &context)?;
    let default_state = string(default.get("state"), "default.state", &context)?;
    if !matches!(default_state, "value" | "required" | "omitted")
        || (default_state == "value") != default.contains_key("value")
    {
        return Err(CatalogError::new(
            "catalog_schema_invalid",
            format!("{context}.default 状态无效"),
        ));
    }
    let unit = object(card.get("unit"), "card.unit", &context)?;
    let unit_state = string(unit.get("state"), "unit.state", &context)?;
    if !matches!(unit_state, "value" | "not_applicable")
        || (unit_state == "value") != unit.contains_key("value")
    {
        return Err(CatalogError::new(
            "catalog_schema_invalid",
            format!("{context}.unit 状态无效"),
        ));
    }
    let range = object(card.get("range"), "card.range", &context)?;
    let range_state = string(range.get("state"), "range.state", &context)?;
    if !matches!(range_state, "bounded" | "not_applicable") {
        return Err(CatalogError::new(
            "catalog_schema_invalid",
            format!("{context}.range 状态无效"),
        ));
    }
    if range_state == "bounded" {
        for required in ["minimum", "maximum", "minimumInclusive", "maximumInclusive"] {
            if !range.contains_key(required) {
                return Err(CatalogError::new(
                    "catalog_schema_invalid",
                    format!("{context}.range 缺少 {required}"),
                ));
            }
        }
    }
    let enum_value = object(card.get("enum"), "card.enum", &context)?;
    let enum_state = string(enum_value.get("state"), "enum.state", &context)?;
    if !matches!(enum_state, "values" | "not_applicable")
        || (enum_state == "values") != enum_value.contains_key("values")
    {
        return Err(CatalogError::new(
            "catalog_schema_invalid",
            format!("{context}.enum 状态无效"),
        ));
    }
    Ok(StructCardContract {
        default: CardDefault {
            state: default_state.to_string(),
            value: default.get("value").cloned(),
        },
        unit: CardUnit {
            state: unit_state.to_string(),
            value: unit
                .get("value")
                .and_then(Value::as_str)
                .map(str::to_string),
        },
        range: CardRange {
            state: range_state.to_string(),
            minimum: range.get("minimum").filter(|v| !v.is_null()).cloned(),
            maximum: range.get("maximum").filter(|v| !v.is_null()).cloned(),
            minimum_inclusive: range.get("minimumInclusive").and_then(Value::as_bool),
            maximum_inclusive: range.get("maximumInclusive").and_then(Value::as_bool),
        },
        enum_values: CardEnum {
            state: enum_state.to_string(),
            values: enum_value
                .get("values")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            case_insensitive: enum_value
                .get("caseInsensitive")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
        visible_when: parse_card_condition(card.get("visibleWhen"), &context)?,
        enabled_when: parse_card_condition(card.get("enabledWhen"), &context)?,
    })
}

fn parse_card_condition(
    value: Option<&Value>,
    context: &str,
) -> Result<CardCondition, CatalogError> {
    let condition = object(value, "card condition", context)?;
    let state = string(condition.get("state"), "condition.state", context)?;
    match state {
        "always" if !condition.contains_key("when") => Ok(CardCondition {
            state: state.to_string(),
            when: None,
        }),
        "condition" => {
            let when = object(condition.get("when"), "condition.when", context)?;
            if !when.contains_key("field")
                || (!when.contains_key("equals") && !when.contains_key("in"))
            {
                return Err(CatalogError::new(
                    "catalog_schema_invalid",
                    format!("{context} 的 condition.when 无效"),
                ));
            }
            Ok(CardCondition {
                state: state.to_string(),
                when: condition.get("when").cloned(),
            })
        }
        _ => Err(CatalogError::new(
            "catalog_schema_invalid",
            format!("{context} 的 condition state 无效"),
        )),
    }
}

fn parse_rule(value: &Value, schema_ref: &str) -> Result<SchemaRule, CatalogError> {
    let rule = object(Some(value), "rule", schema_ref)?;
    let kind = string(rule.get("type"), "rule.type", schema_ref)?;
    let value = value.clone();
    match kind {
        "positive" => Ok(SchemaRule::Positive(value)),
        "nonnegative" => Ok(SchemaRule::Nonnegative(value)),
        "range" => Ok(SchemaRule::Range(value)),
        "compare" => Ok(SchemaRule::Compare(value)),
        "conditional_compare" => Ok(SchemaRule::ConditionalCompare(value)),
        "conditional_required" => Ok(SchemaRule::ConditionalRequired(value)),
        "conditional_enabled" => Ok(SchemaRule::ConditionalEnabled(value)),
        "conditional_ignored" => Ok(SchemaRule::ConditionalIgnored(value)),
        "conditional_empty_input" => Ok(SchemaRule::ConditionalEmptyInput(value)),
        "mutually_exclusive" => Ok(SchemaRule::MutuallyExclusive(value)),
        "mutually_exclusive_groups" => Ok(SchemaRule::MutuallyExclusiveGroups(value)),
        "paired_presence" => Ok(SchemaRule::PairedPresence(value)),
        "length" => Ok(SchemaRule::Length(value)),
        "length_equal" => Ok(SchemaRule::LengthEqual(value)),
        "equal_length" => Ok(SchemaRule::EqualLength(value)),
        "unique" => Ok(SchemaRule::Unique(value)),
        "unique_items" => Ok(SchemaRule::UniqueItems(value)),
        "unique_csv_items" => Ok(SchemaRule::UniqueCsvItems(value)),
        "finite" => Ok(SchemaRule::Finite(value)),
        "requires" => Ok(SchemaRule::Requires(value)),
        "forbidden" => Ok(SchemaRule::Forbidden(value)),
        "enum" => Ok(SchemaRule::Enum(value)),
        "strictly_increasing" => Ok(SchemaRule::StrictlyIncreasing(value)),
        "positive_contiguous" => Ok(SchemaRule::PositiveContiguous(value)),
        "contiguous_zero_based" => Ok(SchemaRule::ContiguousZeroBased(value)),
        "exactly_one_variant" => Ok(SchemaRule::ExactlyOneVariant(value)),
        "cross_table_dense_matrix" => Ok(SchemaRule::CrossTableDenseMatrix(value)),
        _ => Err(CatalogError::new(
            "catalog_schema_invalid",
            format!("{schema_ref} 使用未知 rule: {kind}"),
        )),
    }
}

fn validate_rule_references(
    schema_ref: &str,
    contract: &DataFrameSchemaContract,
) -> Result<(), CatalogError> {
    for rule in &contract.rules {
        let SchemaRule::ContiguousZeroBased(value) = rule else {
            continue;
        };
        let object = value.as_object().ok_or_else(|| {
            CatalogError::new(
                "catalog_schema_invalid",
                format!("{schema_ref} contiguous_zero_based 不是对象"),
            )
        })?;
        if object.len() != 2
            || object.get("type").and_then(Value::as_str) != Some("contiguous_zero_based")
        {
            return Err(CatalogError::new(
                "catalog_schema_invalid",
                format!("{schema_ref} contiguous_zero_based 结构无效"),
            ));
        }
        let field_name = object
            .get("field")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                CatalogError::new(
                    "catalog_schema_invalid",
                    format!("{schema_ref} contiguous_zero_based 缺少 field"),
                )
            })?;
        let field = contract
            .fields
            .iter()
            .find(|field| field.name == field_name)
            .ok_or_else(|| {
                CatalogError::new(
                    "catalog_schema_invalid",
                    format!("{schema_ref} contiguous_zero_based 引用未知字段 {field_name}"),
                )
            })?;
        let unsigned = match &field.dtype {
            DataTypeContract::Primitive { name } | DataTypeContract::Semantic { name } => {
                name.starts_with("u32") || name.starts_with("u64")
            }
            DataTypeContract::Constructor { .. } => false,
        };
        if !unsigned {
            return Err(CatalogError::new(
                "catalog_schema_invalid",
                format!("{schema_ref}.{field_name} 必须是无符号整数"),
            ));
        }
    }
    Ok(())
}

fn parse_discriminator(value: &Value, schema_ref: &str) -> Result<Discriminator, CatalogError> {
    let discriminator = object(Some(value), "discriminator", schema_ref)?;
    let source = string(
        discriminator.get("source"),
        "discriminator.source",
        schema_ref,
    )?;
    if !matches!(source, "column" | "presence" | "external_struct") {
        return Err(CatalogError::new(
            "catalog_schema_invalid",
            format!("{schema_ref} 使用未知 discriminator source: {source}"),
        ));
    }
    Ok(Discriminator {
        field: string(
            discriminator.get("field"),
            "discriminator.field",
            schema_ref,
        )?
        .to_string(),
        source: source.to_string(),
        synthetic: discriminator
            .get("synthetic")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        case_insensitive: discriminator
            .get("caseInsensitive")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        default: discriminator.get("default").cloned(),
    })
}

fn parse_variant(value: &Value, schema_ref: &str) -> Result<SchemaVariant, CatalogError> {
    let variant = object(Some(value), "variant", schema_ref)?;
    let when = variant.get("when").cloned().ok_or_else(|| {
        CatalogError::new(
            "catalog_schema_invalid",
            format!("{schema_ref} variant 缺少 when"),
        )
    })?;
    let when_object = when.as_object().ok_or_else(|| {
        CatalogError::new(
            "catalog_schema_invalid",
            format!("{schema_ref} variant.when 不是对象"),
        )
    })?;
    if !["equals", "notEquals", "in", "present"]
        .iter()
        .any(|key| when_object.contains_key(*key))
    {
        return Err(CatalogError::new(
            "catalog_schema_invalid",
            format!("{schema_ref} variant.when 条件无效"),
        ));
    }
    Ok(SchemaVariant {
        id: string(variant.get("id"), "variant.id", schema_ref)?.to_string(),
        when,
        required: optional_string_array(variant.get("required"), "variant.required", schema_ref)?,
        optional: optional_string_array(variant.get("optional"), "variant.optional", schema_ref)?,
        enabled: optional_string_array(variant.get("enabled"), "variant.enabled", schema_ref)?,
        side_schemas: optional_string_array(
            variant.get("sideSchemas"),
            "variant.sideSchemas",
            schema_ref,
        )?,
        row_count: variant
            .get("rowCount")
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| {
                        CatalogError::new(
                            "catalog_schema_invalid",
                            format!("{schema_ref} variant.rowCount 无效"),
                        )
                    })
                    .and_then(parse_row_count)
            })
            .transpose()?,
    })
}

fn validate_contract(
    schema_ref: &str,
    contract: &DataFrameSchemaContract,
    source: Option<&Value>,
) -> Result<(), CatalogError> {
    if contract.fields.is_empty() {
        return Err(CatalogError::new(
            "catalog_schema_invalid",
            format!("{schema_ref} 没有完整字段合同"),
        ));
    }
    reject_duplicate_field_names(&contract.fields, schema_ref)?;
    if contract.kind == DataFrameContractKind::Struct
        && !matches!(
            contract.row_count,
            RowCountConstraint::Zero | RowCountConstraint::One | RowCountConstraint::ZeroOrOne
        )
    {
        return Err(CatalogError::new(
            "catalog_schema_invalid",
            format!("{schema_ref} Struct rowCount 无效"),
        ));
    }
    match contract.shape {
        FrameShape::Uniform
            if contract.discriminator.is_some() || !contract.variants.is_empty() =>
        {
            return Err(CatalogError::new(
                "catalog_schema_invalid",
                format!("{schema_ref} uniform schema 声明了 discriminator/variants"),
            ));
        }
        FrameShape::Discriminated
            if contract.discriminator.is_none() || contract.variants.is_empty() =>
        {
            return Err(CatalogError::new(
                "catalog_schema_invalid",
                format!("{schema_ref} discriminated schema 缺少 discriminator/variants"),
            ));
        }
        _ => {}
    }
    if source.and_then(Value::as_array).is_none_or(Vec::is_empty) {
        return Err(CatalogError::new(
            "catalog_schema_invalid",
            format!("{schema_ref} 缺少 contract.source"),
        ));
    }
    Ok(())
}

fn reject_duplicate_field_names(
    fields: &[SchemaFieldContract],
    schema_ref: &str,
) -> Result<(), CatalogError> {
    let mut names = BTreeSet::new();
    for field in fields {
        if !names.insert(&field.name) {
            return Err(CatalogError::new(
                "catalog_schema_invalid",
                format!("{schema_ref} 重复字段: {}", field.name),
            ));
        }
    }
    Ok(())
}

fn split_schema_ref(value: &str) -> Result<(&str, &str), CatalogError> {
    let mut parts = value.split('.');
    let section = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    if section.is_empty() || name.is_empty() || parts.next().is_some() {
        return Err(CatalogError::new(
            "catalog_schema_invalid",
            format!("schemaRef 必须是两段式引用: {value}"),
        ));
    }
    Ok((section, name))
}

fn object<'a>(
    value: Option<&'a Value>,
    field: &str,
    context: &str,
) -> Result<&'a Map<String, Value>, CatalogError> {
    value.and_then(Value::as_object).ok_or_else(|| {
        CatalogError::new(
            "catalog_schema_invalid",
            format!("{context} 缺少或无效的 {field}"),
        )
    })
}

fn array<'a>(
    value: Option<&'a Value>,
    field: &str,
    context: &str,
) -> Result<&'a Vec<Value>, CatalogError> {
    value.and_then(Value::as_array).ok_or_else(|| {
        CatalogError::new(
            "catalog_schema_invalid",
            format!("{context} 缺少或无效的 {field}"),
        )
    })
}

fn string<'a>(
    value: Option<&'a Value>,
    field: &str,
    context: &str,
) -> Result<&'a str, CatalogError> {
    value
        .and_then(Value::as_str)
        .filter(|item| !item.is_empty())
        .ok_or_else(|| {
            CatalogError::new(
                "catalog_schema_invalid",
                format!("{context} 缺少或无效的 {field}"),
            )
        })
}

fn boolean(value: Option<&Value>, field: &str, context: &str) -> Result<bool, CatalogError> {
    value.and_then(Value::as_bool).ok_or_else(|| {
        CatalogError::new(
            "catalog_schema_invalid",
            format!("{context} 缺少或无效的 {field}"),
        )
    })
}

fn string_array(
    value: Option<&Value>,
    field: &str,
    context: &str,
) -> Result<Vec<String>, CatalogError> {
    array(value, field, context)?
        .iter()
        .map(|value| {
            value.as_str().map(str::to_string).ok_or_else(|| {
                CatalogError::new(
                    "catalog_schema_invalid",
                    format!("{context}.{field} 包含非字符串"),
                )
            })
        })
        .collect()
}

fn optional_string_array(
    value: Option<&Value>,
    field: &str,
    context: &str,
) -> Result<Vec<String>, CatalogError> {
    value
        .map(|value| string_array(Some(value), field, context))
        .transpose()
        .map(Option::unwrap_or_default)
}

fn string_map(
    value: Option<&Value>,
    field: &str,
    context: &str,
) -> Result<BTreeMap<String, String>, CatalogError> {
    object(value, field, context)?
        .iter()
        .map(|(key, value)| {
            value
                .as_str()
                .map(|value| (key.clone(), value.to_string()))
                .ok_or_else(|| {
                    CatalogError::new(
                        "catalog_schema_invalid",
                        format!("{context}.{field}.{key} 不是字符串"),
                    )
                })
        })
        .collect()
}

fn collect_dtype_names(dtype: &DataTypeContract, names: &mut BTreeSet<String>) {
    match dtype {
        DataTypeContract::Primitive { name } | DataTypeContract::Semantic { name } => {
            names.insert(name.clone());
        }
        DataTypeContract::Constructor { name, element } => {
            names.insert(name.clone());
            collect_dtype_names(element, names);
        }
    }
}

fn conversion_name(value: &UploadConversion) -> &'static str {
    match value {
        UploadConversion::NumericToU32Semantics => "numeric_to_u32_semantics",
        UploadConversion::ScalarToF64List => "scalar_to_f64_list",
        UploadConversion::FlatListToMatrix => "flat_list_to_matrix",
        UploadConversion::ScalarToStringList => "scalar_to_string_list",
    }
}

fn hash_json(value: &Value) -> String {
    hex::encode(Sha256::digest(
        serde_json::to_vec(value).expect("typed catalog JSON is serializable"),
    ))
}

pub(crate) fn normalize_name(name: &str) -> String {
    name.trim_start_matches('\u{feff}').trim().to_lowercase()
}
