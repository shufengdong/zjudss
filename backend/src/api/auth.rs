use axum::Json;
use serde_json::{Value, json};

use crate::auth::CurrentPrincipal;

pub async fn me(CurrentPrincipal(principal): CurrentPrincipal) -> Json<Value> {
    Json(json!({
        "subject": principal.subject,
        "scopeKind": principal.access.kind,
        "scopeKey": principal.access.key,
    }))
}
