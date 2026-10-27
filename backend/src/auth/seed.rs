use std::collections::HashSet;
use std::fmt;

use argon2::password_hash::PasswordHasher;
use argon2::{Algorithm, Argon2, Params, Version};
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use sha2::{Digest, Sha256};

use crate::application::StorageLimits;
use crate::db::models::NewUser;
use crate::db::schema;

pub const INIT_USERS_CSV: &[u8] = include_bytes!("../../resources/init_users.csv");

const MAX_FIELD_CHARS: usize = 128;
const ARGON2_MEMORY_KIB: u32 = 19_456;
const ARGON2_ITERATIONS: u32 = 2;
const ARGON2_PARALLELISM: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserSeedSummary {
    pub user_count: usize,
    pub user_ids: Vec<String>,
    pub seed_sha256: String,
    pub initialized: bool,
}

#[derive(Debug)]
pub struct UserSeedError {
    code: &'static str,
    message: String,
}

impl UserSeedError {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_user_seed",
            message: message.into(),
        }
    }

    fn database(error: impl fmt::Display) -> Self {
        Self {
            code: "user_seed_database_error",
            message: error.to_string(),
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for UserSeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for UserSeedError {}

impl From<diesel::result::Error> for UserSeedError {
    fn from(value: diesel::result::Error) -> Self {
        Self::database(value)
    }
}

#[derive(Debug)]
struct SeedUser {
    id: String,
    name: String,
}

pub fn seed_users(
    conn: &mut SqliteConnection,
    csv_bytes: &[u8],
    now_ms: i64,
    limits: StorageLimits,
) -> Result<UserSeedSummary, UserSeedError> {
    let rows = parse(csv_bytes)?;
    let seed_sha256 = hex::encode(Sha256::digest(csv_bytes));
    let existing = schema::users::table.count().get_result::<i64>(conn)?;
    if existing > 0 {
        let saved = schema::system_metadata::table
            .find("init_users_sha256")
            .select(schema::system_metadata::value)
            .first::<String>(conn)
            .optional()?;
        if saved.as_deref() != Some(seed_sha256.as_str()) {
            return Err(UserSeedError {
                code: "user_seed_mismatch_reset_required",
                message: "内嵌用户清单已变化，请重置数据库".to_string(),
            });
        }
        return Ok(UserSeedSummary {
            user_count: rows.len(),
            user_ids: rows.iter().map(|row| row.id.clone()).collect(),
            seed_sha256,
            initialized: false,
        });
    }

    validate_limits(limits)?;
    conn.immediate_transaction::<_, UserSeedError, _>(|conn| {
        let argon2 = Argon2::new(
            Algorithm::Argon2id,
            Version::V0x13,
            Params::new(
                ARGON2_MEMORY_KIB,
                ARGON2_ITERATIONS,
                ARGON2_PARALLELISM,
                None,
            )
            .map_err(UserSeedError::database)?,
        );
        for row in &rows {
            diesel::insert_into(schema::storage_scopes::table)
                .values((
                    schema::storage_scopes::kind.eq("user"),
                    schema::storage_scopes::scope_key.eq(&row.id),
                    schema::storage_scopes::created_at_ms.eq(now_ms),
                    schema::storage_scopes::artifact_limit.eq(limits.artifact_limit),
                    schema::storage_scopes::byte_limit.eq(limits.byte_limit),
                    schema::storage_scopes::warning_ratio.eq(limits.warning_ratio),
                    schema::storage_scopes::updated_at_ms.eq(now_ms),
                ))
                .execute(conn)?;
            let storage_scope_id = schema::storage_scopes::table
                .filter(schema::storage_scopes::kind.eq("user"))
                .filter(schema::storage_scopes::scope_key.eq(&row.id))
                .select(schema::storage_scopes::id)
                .first::<i32>(conn)?;
            let password_hash = argon2
                .hash_password(row.id.as_bytes())
                .map_err(UserSeedError::database)?
                .to_string();
            diesel::insert_into(schema::users::table)
                .values(NewUser {
                    id: row.id.clone(),
                    name: row.name.clone(),
                    password_hash,
                    storage_scope_id,
                    created_at_ms: now_ms,
                })
                .execute(conn)?;
        }
        diesel::insert_into(schema::system_metadata::table)
            .values((
                schema::system_metadata::key.eq("init_users_sha256"),
                schema::system_metadata::value.eq(&seed_sha256),
            ))
            .execute(conn)?;
        Ok(())
    })?;

    Ok(UserSeedSummary {
        user_count: rows.len(),
        user_ids: rows.iter().map(|row| row.id.clone()).collect(),
        seed_sha256,
        initialized: true,
    })
}

fn parse(csv_bytes: &[u8]) -> Result<Vec<SeedUser>, UserSeedError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(false)
        .from_reader(csv_bytes);
    let headers = reader
        .headers()
        .map_err(|_| UserSeedError::invalid("用户清单无法解析"))?;
    if headers.len() != 2 || headers.get(0) != Some("ID") || headers.get(1) != Some("NAME") {
        return Err(UserSeedError::invalid("用户清单表头必须为 ID,NAME"));
    }

    let mut ids = HashSet::new();
    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|_| UserSeedError::invalid("用户清单无法解析"))?;
        let id = record.get(0).unwrap_or_default().trim().to_string();
        let name = record.get(1).unwrap_or_default().trim().to_string();
        if !valid_field(&id) || !valid_field(&name) || !ids.insert(id.clone()) {
            return Err(UserSeedError::invalid("用户清单包含非法或重复字段"));
        }
        rows.push(SeedUser { id, name });
    }
    if rows.is_empty() {
        return Err(UserSeedError::invalid("用户清单不能为空"));
    }
    Ok(rows)
}

fn valid_field(value: &str) -> bool {
    let len = value.chars().count();
    len > 0 && len <= MAX_FIELD_CHARS && !value.chars().any(char::is_control)
}

fn validate_limits(limits: StorageLimits) -> Result<(), UserSeedError> {
    if limits.artifact_limit <= 0
        || limits.byte_limit <= 0
        || !(0.0 < limits.warning_ratio && limits.warning_ratio <= 1.0)
    {
        return Err(UserSeedError::invalid("用户存储配额无效"));
    }
    Ok(())
}
