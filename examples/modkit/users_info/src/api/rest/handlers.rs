use axum::{extract::Path, http::StatusCode, response::IntoResponse, response::Json, Extension};
use tracing::info;
use uuid::Uuid;

use crate::api::rest::dto::{CreateUserReq, UpdateUserReq, UserDto, UserEvent};

use modkit::api::odata::OData;
use modkit::api::ApiError;
use odata_core::Page;

use crate::domain::service::Service;
use modkit::SseBroadcaster;

// Type alias for our specific ApiError with DomainError
type UsersApiError = ApiError<crate::domain::error::DomainError>;

/// List users with cursor-based pagination
pub async fn list_users(
    Extension(svc): Extension<std::sync::Arc<Service>>,
    OData(query): OData,
) -> Result<Json<Page<UserDto>>, UsersApiError> {
    info!("Listing users with cursor pagination");

    let page = svc.list_users_page(query).await?.map_items(UserDto::from);
    Ok(Json(page))
}

/// Get a specific user by ID
pub async fn get_user(
    Extension(svc): Extension<std::sync::Arc<Service>>,
    Path(id): Path<Uuid>,
) -> Result<Json<UserDto>, UsersApiError> {
    info!("Getting user with id: {}", id);

    let user = svc.get_user(id).await.map_err(UsersApiError::from_domain)?;
    Ok(Json(UserDto::from(user)))
}

/// Create a new user
pub async fn create_user(
    Extension(svc): Extension<std::sync::Arc<Service>>,
    Json(req_body): Json<CreateUserReq>,
) -> Result<(StatusCode, Json<UserDto>), UsersApiError> {
    info!("Creating user: {:?}", req_body);

    let new_user = req_body.into();
    let user = svc
        .create_user(new_user)
        .await
        .map_err(UsersApiError::from_domain)?;
    Ok((StatusCode::CREATED, Json(UserDto::from(user))))
}

/// Update an existing user
pub async fn update_user(
    Extension(svc): Extension<std::sync::Arc<Service>>,
    Path(id): Path<Uuid>,
    Json(req_body): Json<UpdateUserReq>,
) -> Result<Json<UserDto>, UsersApiError> {
    info!("Updating user {} with: {:?}", id, req_body);

    let patch = req_body.into();
    let user = svc
        .update_user(id, patch)
        .await
        .map_err(UsersApiError::from_domain)?;
    Ok(Json(UserDto::from(user)))
}

/// Delete a user by ID
pub async fn delete_user(
    Extension(svc): Extension<std::sync::Arc<Service>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, UsersApiError> {
    info!("Deleting user: {}", id);

    svc.delete_user(id)
        .await
        .map_err(UsersApiError::from_domain)?;
    Ok(StatusCode::NO_CONTENT)
}

/// SSE endpoint returning a live stream of `UserEvent`.
pub async fn users_events(
    Extension(sse): Extension<SseBroadcaster<UserEvent>>,
) -> impl IntoResponse {
    info!("New SSE connection for user events");
    sse.sse_response_named("users_events")
}
