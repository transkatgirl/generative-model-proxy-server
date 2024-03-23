use axum::{
    extract::{Path, State},
    http::StatusCode,
    middleware,
    response::Html,
    routing::get,
    Extension, Json, Router,
};

use uuid::Uuid;

use super::{
    super::AppState,
    state::{DatabaseActionResult, DatabaseLinkedInsertionResult, DatabaseValueResult},
    Authenticated, Model, Quota, Role, User,
};

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route(
            "/users",
            get(get_users).post(add_user_post).put(add_user_put),
        )
        .route(
            "/users/:uuid",
            get(get_user).put(update_user).delete(delete_user),
        )
        .route(
            "/roles",
            get(get_roles).post(add_role_post).put(add_role_put),
        )
        .route(
            "/roles/:uuid",
            get(get_role).put(update_role).delete(delete_role),
        )
        .route(
            "/models",
            get(get_models).post(add_model_post).put(add_model_put),
        )
        .route(
            "/models/:uuid",
            get(get_model).put(update_model).delete(delete_model),
        )
        .route(
            "/quotas",
            get(get_quotas).post(add_quota_post).put(add_quota_put),
        )
        .route(
            "/quotas/:uuid",
            get(get_quota).put(update_quota).delete(delete_quota),
        )
        .route("/help", get(help_page))
        .fallback(StatusCode::NOT_FOUND)
        .layer(middleware::from_fn(super::authenticate_admin))
}

async fn help_page(Extension(auth): Extension<Authenticated>) -> Html<&'static str> {
    if auth.user.uuid == Uuid::default() {
        Html(include_str!("setup-instructions.html"))
    } else {
        Html(include_str!("manual.html"))
    }
}

impl From<DatabaseActionResult> for StatusCode {
    fn from(value: DatabaseActionResult) -> Self {
        match value {
            DatabaseActionResult::Success => StatusCode::OK,
            DatabaseActionResult::NotFound => StatusCode::NOT_FOUND,
            DatabaseActionResult::BackendError => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<DatabaseLinkedInsertionResult> for StatusCode {
    fn from(value: DatabaseLinkedInsertionResult) -> Self {
        match value {
            DatabaseLinkedInsertionResult::Success => StatusCode::OK,
            DatabaseLinkedInsertionResult::Duplicate => StatusCode::CONFLICT,
            DatabaseLinkedInsertionResult::BackendError => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl<T> From<DatabaseValueResult<T>> for Result<Json<T>, StatusCode> {
    fn from(value: DatabaseValueResult<T>) -> Self {
        match value {
            DatabaseValueResult::Success(result) => Ok(Json(result)),
            DatabaseValueResult::NotFound => Err(StatusCode::NOT_FOUND),
            DatabaseValueResult::BackendError => Err(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }
}

async fn get_users(State(state): State<AppState>) -> Result<Json<Vec<User>>, StatusCode> {
    state.database.get_table("users").into()
}

async fn get_user(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<User>, StatusCode> {
    if uuid == Uuid::default() {
        return Err(StatusCode::BAD_REQUEST);
    }

    state.database.get_item("users", &uuid).into()
}

async fn add_user_post(
    State(state): State<AppState>,
    Json(mut payload): Json<User>,
) -> Result<Json<Uuid>, StatusCode> {
    if payload.uuid != Uuid::default() {
        return Err(StatusCode::BAD_REQUEST);
    }
    payload.uuid = Uuid::new_v4();

    let related_items: Vec<_> = payload
        .api_keys
        .iter()
        .map(|item| (item, payload.uuid))
        .collect();

    match state.database.insert_related_items(
        ("users", "api_keys"),
        (&payload.uuid, &payload),
        &related_items,
    ) {
        DatabaseLinkedInsertionResult::Success => Ok(Json(payload.uuid)),
        DatabaseLinkedInsertionResult::Duplicate => Err(StatusCode::CONFLICT),
        DatabaseLinkedInsertionResult::BackendError => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn add_user_put(State(state): State<AppState>, Json(payload): Json<User>) -> StatusCode {
    if payload.uuid == Uuid::default() {
        return StatusCode::BAD_REQUEST;
    }

    let related_items: Vec<_> = payload
        .api_keys
        .iter()
        .map(|item| (item, payload.uuid))
        .collect();

    state
        .database
        .insert_related_items(
            ("users", "api_keys"),
            (&payload.uuid, &payload),
            &related_items,
        )
        .into()
}

async fn update_user(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
    Json(mut payload): Json<User>,
) -> StatusCode {
    if (payload.uuid != Uuid::default() && payload.uuid != uuid) || uuid == Uuid::default() {
        return StatusCode::BAD_REQUEST;
    }
    payload.uuid = uuid;

    let related_items: Vec<_> = payload
        .api_keys
        .iter()
        .map(|item| (item, payload.uuid))
        .collect();

    state
        .database
        .insert_related_items(
            ("users", "api_keys"),
            (&payload.uuid, &payload),
            &related_items,
        )
        .into()
}

async fn delete_user(State(state): State<AppState>, Path(uuid): Path<Uuid>) -> StatusCode {
    if uuid == Uuid::default() {
        return StatusCode::BAD_REQUEST;
    }

    state
        .database
        .remove_related_items::<_, User>(("users", "api_keys"), &uuid)
        .into()
}

async fn get_roles(State(state): State<AppState>) -> Result<Json<Vec<Role>>, StatusCode> {
    state.database.get_table("roles").into()
}

async fn get_role(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Role>, StatusCode> {
    if uuid == Uuid::default() {
        return Err(StatusCode::BAD_REQUEST);
    }

    state.database.get_item("roles", &uuid).into()
}

async fn add_role_post(
    State(state): State<AppState>,
    Json(mut payload): Json<Role>,
) -> Result<Json<Uuid>, StatusCode> {
    if payload.uuid != Uuid::default() {
        return Err(StatusCode::BAD_REQUEST);
    }
    payload.uuid = Uuid::new_v4();

    match state.database.insert_item("roles", &payload.uuid, &payload) {
        DatabaseActionResult::Success => Ok(Json(payload.uuid)),
        DatabaseActionResult::NotFound => Err(StatusCode::NOT_FOUND),
        DatabaseActionResult::BackendError => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn add_role_put(State(state): State<AppState>, Json(payload): Json<Role>) -> StatusCode {
    if payload.uuid == Uuid::default() {
        return StatusCode::BAD_REQUEST;
    }

    state
        .database
        .insert_item("roles", &payload.uuid, &payload)
        .into()
}

async fn update_role(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
    Json(mut payload): Json<Role>,
) -> StatusCode {
    if (payload.uuid != Uuid::default() && payload.uuid != uuid) || uuid == Uuid::default() {
        return StatusCode::BAD_REQUEST;
    }
    payload.uuid = uuid;

    state
        .database
        .insert_item("roles", &payload.uuid, &payload)
        .into()
}

async fn delete_role(State(state): State<AppState>, Path(uuid): Path<Uuid>) -> StatusCode {
    if uuid == Uuid::default() {
        return StatusCode::BAD_REQUEST;
    }

    state.database.remove_item("roles", &uuid).into()
}

async fn get_models(State(state): State<AppState>) -> Result<Json<Vec<Model>>, StatusCode> {
    state.database.get_table("models").into()
}

async fn get_model(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Model>, StatusCode> {
    if uuid == Uuid::default() {
        return Err(StatusCode::BAD_REQUEST);
    }

    state.database.get_item("models", &uuid).into()
}

async fn add_model_post(
    State(state): State<AppState>,
    Json(mut payload): Json<Model>,
) -> Result<Json<Uuid>, StatusCode> {
    if payload.uuid != Uuid::default() {
        return Err(StatusCode::BAD_REQUEST);
    }
    payload.uuid = Uuid::new_v4();

    match state
        .database
        .insert_item("models", &payload.uuid, &payload)
    {
        DatabaseActionResult::Success => Ok(Json(payload.uuid)),
        DatabaseActionResult::NotFound => Err(StatusCode::NOT_FOUND),
        DatabaseActionResult::BackendError => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn add_model_put(State(state): State<AppState>, Json(payload): Json<Model>) -> StatusCode {
    if payload.uuid == Uuid::default() {
        return StatusCode::BAD_REQUEST;
    }

    state
        .database
        .insert_item("models", &payload.uuid, &payload)
        .into()
}

async fn update_model(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
    Json(mut payload): Json<Model>,
) -> StatusCode {
    if (payload.uuid != Uuid::default() && payload.uuid != uuid) || uuid == Uuid::default() {
        return StatusCode::BAD_REQUEST;
    }
    payload.uuid = uuid;

    state
        .database
        .insert_item("models", &payload.uuid, &payload)
        .into()
}

async fn delete_model(State(state): State<AppState>, Path(uuid): Path<Uuid>) -> StatusCode {
    if uuid == Uuid::default() {
        return StatusCode::BAD_REQUEST;
    }

    state.database.remove_item("models", &uuid).into()
}

async fn get_quotas(State(state): State<AppState>) -> Result<Json<Vec<Quota>>, StatusCode> {
    state.database.get_table("quotas").into()
}

async fn get_quota(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Quota>, StatusCode> {
    if uuid == Uuid::default() {
        return Err(StatusCode::BAD_REQUEST);
    }

    state.database.get_item("quotas", &uuid).into()
}

async fn add_quota_post(
    State(state): State<AppState>,
    Json(mut payload): Json<Quota>,
) -> Result<Json<Uuid>, StatusCode> {
    if payload.uuid != Uuid::default() {
        return Err(StatusCode::BAD_REQUEST);
    }
    payload.uuid = Uuid::new_v4();

    match state
        .database
        .insert_item("quotas", &payload.uuid, &payload)
    {
        DatabaseActionResult::Success => Ok(Json(payload.uuid)),
        DatabaseActionResult::NotFound => Err(StatusCode::NOT_FOUND),
        DatabaseActionResult::BackendError => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn add_quota_put(State(state): State<AppState>, Json(payload): Json<Quota>) -> StatusCode {
    if payload.uuid == Uuid::default() {
        return StatusCode::BAD_REQUEST;
    }

    state
        .database
        .insert_item("quotas", &payload.uuid, &payload)
        .into()
}

async fn update_quota(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
    Json(mut payload): Json<Quota>,
) -> StatusCode {
    if (payload.uuid != Uuid::default() && payload.uuid != uuid) || uuid == Uuid::default() {
        return StatusCode::BAD_REQUEST;
    }
    payload.uuid = uuid;

    state
        .database
        .insert_item("quotas", &payload.uuid, &payload)
        .into()
}

async fn delete_quota(State(state): State<AppState>, Path(uuid): Path<Uuid>) -> StatusCode {
    if uuid == Uuid::default() {
        return StatusCode::BAD_REQUEST;
    }

    state.database.remove_item("quotas", &uuid).into()
}
