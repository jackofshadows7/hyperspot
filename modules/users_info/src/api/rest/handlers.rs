

use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::Json,
    Extension,
};
use tracing::{error, info};
use uuid::Uuid;

use crate::api::rest::dto::{CreateUserReq, ListUsersQuery, UpdateUserReq, UserDto, UserListDto};

use crate::domain::service::Service;

/// List users with optional pagination
pub async fn list_users(
    Extension(svc): Extension<std::sync::Arc<Service>>,
    Query(query): Query<ListUsersQuery>,
) -> Result<Json<UserListDto>, StatusCode> {
    info!("Listing users with query: {:?}", query);

    match svc.list_users(query.limit, query.offset).await {
        Ok(users) => {
            let dto_users: Vec<UserDto> = users.into_iter().map(UserDto::from).collect();
            let response = UserListDto {
                total: dto_users.len(),
                limit: query.limit.unwrap_or(50),
                offset: query.offset.unwrap_or(0),
                users: dto_users,
            };
            Ok(Json(response))
        }
        Err(e) => {
            error!("Failed to list users: {}", e);
            Err(map_domain_error_to_status_code(&e))
        }
    }
}

/// Get a specific user by ID
pub async fn get_user(
    Extension(svc): Extension<std::sync::Arc<Service>>,
    Path(id): Path<Uuid>,
) -> Result<Json<UserDto>, StatusCode> {
    info!("Getting user with id: {}", id);

    match svc.get_user(id).await {
        Ok(user) => Ok(Json(UserDto::from(user))),
        Err(e) => {
            error!("Failed to get user {}: {}", id, e);
            Err(map_domain_error_to_status_code(&e))
        }
    }
}

/// Create a new user
pub async fn create_user(
    Extension(svc): Extension<std::sync::Arc<Service>>,
    Json(req): Json<CreateUserReq>,
) -> Result<(StatusCode, Json<UserDto>), StatusCode> {
    info!("Creating user: {:?}", req);

    let new_user = req.into();

    match svc.create_user(new_user).await {
        Ok(user) => Ok((StatusCode::CREATED, Json(UserDto::from(user)))),
        Err(e) => {
            error!("Failed to create user: {}", e);
            Err(map_domain_error_to_status_code(&e))
        }
    }
}

/// Update an existing user
pub async fn update_user(
    Extension(svc): Extension<std::sync::Arc<Service>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateUserReq>,
) -> Result<Json<UserDto>, StatusCode> {
    info!("Updating user {} with: {:?}", id, req);

    let patch = req.into();

    match svc.update_user(id, patch).await {
        Ok(user) => Ok(Json(UserDto::from(user))),
        Err(e) => {
            error!("Failed to update user {}: {}", id, e);
            Err(map_domain_error_to_status_code(&e))
        }
    }
}

/// Delete a user by ID
pub async fn delete_user(
    Extension(svc): Extension<std::sync::Arc<Service>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    info!("Deleting user: {}", id);

    match svc.delete_user(id).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(e) => {
            error!("Failed to delete user {}: {}", id, e);
            Err(map_domain_error_to_status_code(&e))
        }
    }
}

/// Map domain errors to HTTP status codes
fn map_domain_error_to_status_code(error: &crate::domain::error::DomainError) -> StatusCode {
    use crate::domain::error::DomainError;

    match error {
        DomainError::UserNotFound { .. } => StatusCode::NOT_FOUND,
        DomainError::EmailAlreadyExists { .. } => StatusCode::CONFLICT,
        DomainError::InvalidEmail { .. }
        | DomainError::EmptyDisplayName
        | DomainError::DisplayNameTooLong { .. }
        | DomainError::Validation { .. } => StatusCode::BAD_REQUEST,
        DomainError::Database { .. } => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
