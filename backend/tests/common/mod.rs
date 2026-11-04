#![allow(dead_code, unused_imports)]

mod app;
mod fixtures;
mod http;

pub use app::TestApp;
pub use fixtures::{publish_model_bytes, seed_test_users, test_user};
pub use http::session_deletion_token;

#[derive(diesel::QueryableByName)]
pub struct CountRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub count: i64,
}
