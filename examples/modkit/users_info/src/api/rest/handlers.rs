use axum::{
    extract::{Path, Query},
    http::{StatusCode, Uri},
    response::IntoResponse,
    response::Json,
    Extension,
};
use tracing::{error, info};
use uuid::Uuid;

use crate::api::rest::dto::{
    CreateUserReq, ListUsersQuery, UpdateUserReq, UserDto, UserEvent, UserListDto,
};

use modkit::api::odata::OData;

use crate::api::rest::error::map_domain_error;
use crate::domain::service::Service;
use modkit::{api::problem::ProblemResponse, SseBroadcaster};

pub async fn list_users(
    Extension(svc): Extension<std::sync::Arc<Service>>,
    Query(query): Query<ListUsersQuery>,
    OData(filter): OData,
    uri: Uri,
) -> Result<axum::Json<UserListDto>, ProblemResponse> {
    info!("Listing users with query: {:?}", query);

    match svc.list_users(filter, query.limit, query.offset).await {
        Ok(users) => {
            let dto_users: Vec<UserDto> = users.into_iter().map(UserDto::from).collect();
            let response = UserListDto {
                total: dto_users.len(),
                limit: query.limit.unwrap_or(50),
                offset: query.offset.unwrap_or(0),
                users: dto_users,
            };
            Ok(axum::Json(response))
        }
        Err(e) => {
            error!("Failed to list users: {}", e);
            Err(map_domain_error(&e, uri.path()))
        }
    }
}

/// Get a specific user by ID
pub async fn get_user(
    Extension(svc): Extension<std::sync::Arc<Service>>,
    Path(id): Path<Uuid>,
    uri: Uri,
) -> Result<Json<UserDto>, ProblemResponse> {
    info!("Getting user with id: {}", id);

    match svc.get_user(id).await {
        Ok(user) => Ok(Json(UserDto::from(user))),
        Err(e) => {
            error!("Failed to get user {}: {}", id, e);
            Err(map_domain_error(&e, uri.path()))
        }
    }
}

/// Create a new user
pub async fn create_user(
    uri: Uri,
    Extension(svc): Extension<std::sync::Arc<Service>>,
    Json(req_body): Json<CreateUserReq>,
) -> Result<(StatusCode, Json<UserDto>), ProblemResponse> {
    info!("Creating user: {:?}", req_body);

    let new_user = req_body.into();

    match svc.create_user(new_user).await {
        Ok(user) => Ok((StatusCode::CREATED, Json(UserDto::from(user)))),
        Err(e) => {
            error!("Failed to create user: {}", e);
            Err(map_domain_error(&e, uri.path()))
        }
    }
}

/// Update an existing user
pub async fn update_user(
    uri: Uri,
    Extension(svc): Extension<std::sync::Arc<Service>>,
    Path(id): Path<Uuid>,
    Json(req_body): Json<UpdateUserReq>,
) -> Result<Json<UserDto>, ProblemResponse> {
    info!("Updating user {} with: {:?}", id, req_body);

    let patch = req_body.into();

    match svc.update_user(id, patch).await {
        Ok(user) => Ok(Json(UserDto::from(user))),
        Err(e) => {
            error!("Failed to update user {}: {}", id, e);
            Err(map_domain_error(&e, uri.path()))
        }
    }
}

/// Delete a user by ID
pub async fn delete_user(
    Extension(svc): Extension<std::sync::Arc<Service>>,
    Path(id): Path<Uuid>,
    uri: Uri,
) -> Result<StatusCode, ProblemResponse> {
    info!("Deleting user: {}", id);

    match svc.delete_user(id).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(e) => {
            error!("Failed to delete user {}: {}", id, e);
            Err(map_domain_error(&e, uri.path()))
        }
    }
}

/// SSE endpoint returning a live stream of `UserEvent`.
pub async fn users_events(
    Extension(sse): Extension<SseBroadcaster<UserEvent>>,
) -> impl IntoResponse {
    info!("New SSE connection for user events");
    sse.sse_response_named("users_events")
}
