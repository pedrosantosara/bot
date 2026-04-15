use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
    Json,
};
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Shared set of valid session tokens
pub type SessionStore = Arc<Mutex<HashSet<String>>>;

pub fn new_session_store() -> SessionStore {
    Arc::new(Mutex::new(HashSet::new()))
}

/// Login request
#[derive(Deserialize)]
pub struct LoginBody {
    pub password: String,
}

/// Login handler — validates password and returns a session token
pub async fn login(
    State((store, password)): State<(SessionStore, String)>,
    Json(body): Json<LoginBody>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if body.password != password {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let token = uuid::Uuid::new_v4().to_string();
    store.lock().await.insert(token.clone());
    Ok(Json(serde_json::json!({ "token": token })))
}

/// Middleware that checks for a valid session token in the Authorization header
pub async fn require_auth(
    State((store, _password)): State<(SessionStore, String)>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let token = req.headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim_start_matches("Bearer ").to_string());

    match token {
        Some(t) if store.lock().await.contains(&t) => Ok(next.run(req).await),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}
