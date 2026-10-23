use std::fmt::Write as _;
use std::time::Duration;

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordVerifier};
use axum::http::{HeaderMap, header};
use cookie::{Cookie, SameSite, time::Duration as CookieDuration};
use diesel::prelude::*;
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;

use crate::db::models::{AuthSession, User};
use crate::db::{DbPool, schema};

const COOKIE_NAME: &str = "zjudss_session";

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct UserId(pub String);

#[derive(Clone, Debug)]
pub struct CurrentUser {
    pub id: UserId,
    pub name: String,
    pub storage_scope_id: i32,
    pub auth_session_hash: String,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicUser {
    pub id: String,
    pub name: String,
}

#[derive(Debug)]
pub struct LoginSuccess {
    pub user: PublicUser,
    pub set_cookie: String,
}

#[derive(Debug)]
pub struct Authentication {
    pub user: CurrentUser,
    pub renewal_cookie: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthError {
    InvalidCredentials,
    AuthenticationRequired,
    Database,
}

#[derive(Clone)]
pub struct AuthService {
    pool: DbPool,
    ttl_ms: i64,
    secure_cookie: bool,
    revocations: broadcast::Sender<String>,
}

impl AuthService {
    pub fn new(pool: DbPool, ttl: Duration, secure_cookie: bool) -> Self {
        let ttl_ms = i64::try_from(ttl.as_millis()).expect("session TTL fits i64 milliseconds");
        let (revocations, _) = broadcast::channel(128);
        Self {
            pool,
            ttl_ms,
            secure_cookie,
            revocations,
        }
    }

    pub async fn login(&self, id: String, password: String) -> Result<LoginSuccess, AuthError> {
        let pool = self.pool.clone();
        let lookup_id = id.trim().to_owned();
        let user = tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(|_| AuthError::Database)?;
            schema::users::table
                .find(lookup_id)
                .first::<User>(&mut conn)
                .optional()
                .map_err(|_| AuthError::Database)
        })
        .await
        .map_err(|_| AuthError::Database)??
        .ok_or(AuthError::InvalidCredentials)?;
        let parsed = PasswordHash::new(&user.password_hash).map_err(|_| AuthError::Database)?;
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .map_err(|_| AuthError::InvalidCredentials)?;

        let token = random_token();
        let token_hash = token_hash(&token);
        let now_ms = crate::application::now_ms();
        let expires_at_ms = now_ms.checked_add(self.ttl_ms).ok_or(AuthError::Database)?;
        let session = AuthSession {
            token_hash,
            user_id: user.id.clone(),
            created_at_ms: now_ms,
            last_seen_at_ms: now_ms,
            expires_at_ms,
        };
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(|_| AuthError::Database)?;
            diesel::insert_into(schema::auth_sessions::table)
                .values(session)
                .execute(&mut conn)
                .map_err(|_| AuthError::Database)?;
            Ok::<_, AuthError>(())
        })
        .await
        .map_err(|_| AuthError::Database)??;

        Ok(LoginSuccess {
            user: PublicUser {
                id: user.id,
                name: user.name,
            },
            set_cookie: self.session_cookie(&token),
        })
    }

    pub async fn authenticate(&self, headers: &HeaderMap) -> Result<Authentication, AuthError> {
        let token = read_token(headers).ok_or(AuthError::AuthenticationRequired)?;
        let hash = token_hash(&token);
        let pool = self.pool.clone();
        let lookup_hash = hash.clone();
        let now_ms = crate::application::now_ms();
        let ttl_ms = self.ttl_ms;
        let record = tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(|_| AuthError::Database)?;
            let record = schema::auth_sessions::table
                .inner_join(schema::users::table)
                .filter(schema::auth_sessions::token_hash.eq(&lookup_hash))
                .select((
                    schema::auth_sessions::all_columns,
                    schema::users::all_columns,
                ))
                .first::<(AuthSession, User)>(&mut conn)
                .optional()
                .map_err(|_| AuthError::Database)?;
            let Some((mut session, user)) = record else {
                return Ok(None);
            };
            if session.expires_at_ms <= now_ms {
                diesel::delete(schema::auth_sessions::table.find(&lookup_hash))
                    .execute(&mut conn)
                    .map_err(|_| AuthError::Database)?;
                return Ok(None);
            }
            let renew = session.expires_at_ms - now_ms <= ttl_ms / 2;
            if renew {
                session.last_seen_at_ms = now_ms;
                session.expires_at_ms = now_ms.checked_add(ttl_ms).ok_or(AuthError::Database)?;
                diesel::update(schema::auth_sessions::table.find(&lookup_hash))
                    .set((
                        schema::auth_sessions::last_seen_at_ms.eq(session.last_seen_at_ms),
                        schema::auth_sessions::expires_at_ms.eq(session.expires_at_ms),
                    ))
                    .execute(&mut conn)
                    .map_err(|_| AuthError::Database)?;
            }
            Ok(Some((session, user, renew)))
        })
        .await
        .map_err(|_| AuthError::Database)??
        .ok_or(AuthError::AuthenticationRequired)?;
        let (_session, user, renewed) = record;
        Ok(Authentication {
            user: CurrentUser {
                id: UserId(user.id),
                name: user.name,
                storage_scope_id: user.storage_scope_id,
                auth_session_hash: hash,
            },
            renewal_cookie: renewed.then(|| self.session_cookie(&token)),
        })
    }

    pub async fn logout(&self, headers: &HeaderMap) -> Result<String, AuthError> {
        if let Some(token) = read_token(headers) {
            let hash = token_hash(&token);
            let pool = self.pool.clone();
            let delete_hash = hash.clone();
            tokio::task::spawn_blocking(move || {
                let mut conn = pool.get().map_err(|_| AuthError::Database)?;
                diesel::delete(schema::auth_sessions::table.find(delete_hash))
                    .execute(&mut conn)
                    .map_err(|_| AuthError::Database)?;
                Ok::<_, AuthError>(())
            })
            .await
            .map_err(|_| AuthError::Database)??;
            let _ = self.revocations.send(hash);
        }
        Ok(self.expired_cookie())
    }

    pub fn subscribe_revocations(&self) -> broadcast::Receiver<String> {
        self.revocations.subscribe()
    }

    pub async fn is_session_active(&self, token_hash: &str) -> Result<bool, AuthError> {
        let pool = self.pool.clone();
        let token_hash = token_hash.to_owned();
        let now_ms = crate::application::now_ms();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(|_| AuthError::Database)?;
            schema::auth_sessions::table
                .find(token_hash)
                .filter(schema::auth_sessions::expires_at_ms.gt(now_ms))
                .select(schema::auth_sessions::token_hash)
                .first::<String>(&mut conn)
                .optional()
                .map(|row| row.is_some())
                .map_err(|_| AuthError::Database)
        })
        .await
        .map_err(|_| AuthError::Database)?
    }

    pub fn cleanup_expired(&self, now_ms: i64, limit: i64) -> Result<usize, AuthError> {
        let mut conn = self.pool.get().map_err(|_| AuthError::Database)?;
        let hashes = schema::auth_sessions::table
            .filter(schema::auth_sessions::expires_at_ms.le(now_ms))
            .select(schema::auth_sessions::token_hash)
            .limit(limit.clamp(1, 10_000))
            .load::<String>(&mut conn)
            .map_err(|_| AuthError::Database)?;
        diesel::delete(
            schema::auth_sessions::table.filter(schema::auth_sessions::token_hash.eq_any(hashes)),
        )
        .execute(&mut conn)
        .map_err(|_| AuthError::Database)
    }

    fn session_cookie(&self, token: &str) -> String {
        Cookie::build((COOKIE_NAME, token.to_owned()))
            .http_only(true)
            .same_site(SameSite::Lax)
            .path("/")
            .secure(self.secure_cookie)
            .max_age(CookieDuration::milliseconds(self.ttl_ms))
            .build()
            .to_string()
    }

    fn expired_cookie(&self) -> String {
        Cookie::build((COOKIE_NAME, ""))
            .http_only(true)
            .same_site(SameSite::Lax)
            .path("/")
            .secure(self.secure_cookie)
            .max_age(CookieDuration::ZERO)
            .build()
            .to_string()
    }
}

fn read_token(headers: &HeaderMap) -> Option<String> {
    headers.get_all(header::COOKIE).iter().find_map(|value| {
        value.to_str().ok()?.split(';').find_map(|part| {
            let cookie = Cookie::parse(part.trim().to_owned()).ok()?;
            (cookie.name() == COOKIE_NAME).then(|| cookie.value().to_owned())
        })
    })
}

fn token_hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let mut token = String::with_capacity(64);
    for byte in bytes {
        write!(&mut token, "{byte:02x}").expect("write to string");
    }
    token
}
