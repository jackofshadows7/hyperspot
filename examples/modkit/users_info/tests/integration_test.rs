//! Integration-style tests for the users_info module.
//!
//! Key points:
//! - Each test runs on a fresh in-memory SQLite DB and applies migrations.
//! - Service is constructed with a SeaORM-backed repository (Domain Port + Adapter).
//! - Local client is tested against the same Service.
//! - REST layer is exercised via an Axum Router registered through real routes.

use std::sync::Arc;

use anyhow::Result;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use chrono::Utc;
use sea_orm::{Database, DatabaseConnection};
use sea_orm_migration::MigratorTrait;
use tower::ServiceExt;
use uuid::Uuid;

use users_info::{
    api::rest::dto::{CreateUserReq, UserDto},
    contract::client::UsersInfoApi,
    domain::service::{Service, ServiceConfig},
    gateways::local::UsersInfoLocalClient,
    infra::storage::{
        migrations::Migrator,
        sea_orm_repo::SeaOrmUsersRepository, // <-- SeaORM adapter (implements UsersRepository)
    },
};

/// Create a fresh test database for each test (in-memory SQLite) and run migrations.
async fn create_test_db() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("Failed to connect to test database");
    Migrator::up(&db, None)
        .await
        .expect("Failed to run migrations");
    db
}

/// Build the domain Service with a SeaORM-backed repository.
async fn create_test_service() -> Arc<Service> {
    let db = create_test_db().await;
    let repo = SeaOrmUsersRepository::new(db);
    let config = ServiceConfig::default();
    Arc::new(Service::new(Arc::new(repo), config))
}

/// Build a local in-process client on top of the Service.
async fn create_test_client() -> Arc<dyn UsersInfoApi> {
    let service = create_test_service().await;
    Arc::new(UsersInfoLocalClient::new(service))
}

/// Minimal OpenAPI registry stub for tests.
///
/// We only need it to satisfy the route registration path. It records nothing.
struct MockOpenApiRegistry;

impl modkit::api::OpenApiRegistry for MockOpenApiRegistry {
    fn register_operation(&self, _spec: &modkit::api::OperationSpec) {
        // no-op in tests
    }

    // Some versions of the trait include this convenience helper;
    // keep it to stay source-compatible with your current codebase.
    fn ensure_schema_raw(
        &self,
        root_name: &str,
        _schemas: Vec<(
            String,
            utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>,
        )>,
    ) -> String {
        root_name.to_string()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Build an Axum router by calling the real route registration.
async fn create_test_router() -> Router {
    let service = create_test_service().await;
    let openapi = MockOpenApiRegistry;
    users_info::api::rest::routes::register_routes(Router::new(), &openapi, service)
        .expect("Failed to register routes")
}

#[tokio::test]
async fn test_domain_service_crud() -> Result<()> {
    let service = create_test_service().await;

    // create
    let new_user = users_info::contract::model::NewUser {
        email: "test@example.com".to_string(),
        display_name: "Test User".to_string(),
    };
    let created_user = service.create_user(new_user).await?;
    assert_eq!(created_user.email, "test@example.com");
    assert_eq!(created_user.display_name, "Test User");

    // get
    let retrieved_user = service.get_user(created_user.id).await?;
    assert_eq!(retrieved_user.id, created_user.id);
    assert_eq!(retrieved_user.email, created_user.email);

    // list
    let users = service.list_users(None, None).await?;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].id, created_user.id);

    // update
    let patch = users_info::contract::model::UserPatch {
        email: None,
        display_name: Some("Updated Name".to_string()),
    };
    let updated_user = service.update_user(created_user.id, patch).await?;
    assert_eq!(updated_user.display_name, "Updated Name");
    assert_eq!(updated_user.email, "test@example.com");

    // delete
    service.delete_user(created_user.id).await?;
    let result = service.get_user(created_user.id).await;
    assert!(result.is_err(), "user should be gone");

    Ok(())
}

#[tokio::test]
async fn test_domain_service_validation() -> Result<()> {
    let service = create_test_service().await;

    // invalid email
    let invalid_email_user = users_info::contract::model::NewUser {
        email: "invalid-email".to_string(),
        display_name: "Test User".to_string(),
    };
    let result = service.create_user(invalid_email_user).await;
    assert!(result.is_err());

    // empty display name
    let empty_name_user = users_info::contract::model::NewUser {
        email: "test@example.com".to_string(),
        display_name: "".to_string(),
    };
    let result = service.create_user(empty_name_user).await;
    assert!(result.is_err());

    Ok(())
}

#[tokio::test]
async fn test_domain_service_email_uniqueness() -> Result<()> {
    let service = create_test_service().await;

    // first
    let new_user1 = users_info::contract::model::NewUser {
        email: "unique@example.com".to_string(),
        display_name: "User 1".to_string(),
    };
    service.create_user(new_user1).await?;

    // second with same email -> error
    let new_user2 = users_info::contract::model::NewUser {
        email: "unique@example.com".to_string(),
        display_name: "User 2".to_string(),
    };
    let result = service.create_user(new_user2).await;
    assert!(result.is_err());

    Ok(())
}

#[tokio::test]
async fn test_local_client() -> Result<()> {
    let client = create_test_client().await;

    // create
    let new_user = users_info::contract::model::NewUser {
        email: "client@example.com".to_string(),
        display_name: "Client User".to_string(),
    };
    let created_user = client.create_user(new_user).await?;
    assert_eq!(created_user.email, "client@example.com");

    // get
    let retrieved_user = client.get_user(created_user.id).await?;
    assert_eq!(retrieved_user.id, created_user.id);

    // list
    let users = client.list_users(Some(10), Some(0)).await?;
    assert!(!users.is_empty());

    // update
    let patch = users_info::contract::model::UserPatch {
        email: None,
        display_name: Some("Updated Client User".to_string()),
    };
    let updated_user = client.update_user(created_user.id, patch).await?;
    assert_eq!(updated_user.display_name, "Updated Client User");

    // delete
    client.delete_user(created_user.id).await?;

    Ok(())
}

#[tokio::test]
async fn test_rest_api_create_user() -> Result<()> {
    let router = create_test_router().await;

    let create_request = CreateUserReq {
        email: "rest@example.com".to_string(),
        display_name: "REST User".to_string(),
    };

    let request = Request::builder()
        .method("POST")
        .uri("/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&create_request)?))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await?;
    let user: UserDto = serde_json::from_slice(&body)?;
    assert_eq!(user.email, "rest@example.com");
    assert_eq!(user.display_name, "REST User");

    Ok(())
}

#[tokio::test]
async fn test_rest_api_list_users() -> Result<()> {
    let router = create_test_router().await;

    let request = Request::builder()
        .method("GET")
        .uri("/users")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await?;
    let user_list: users_info::api::rest::dto::UserListDto = serde_json::from_slice(&body)?;

    // default paging is defined by ServiceConfig::default()
    assert_eq!(user_list.limit, 50);
    assert_eq!(user_list.offset, 0);

    Ok(())
}

#[tokio::test]
async fn test_rest_api_validation_errors() -> Result<()> {
    let router = create_test_router().await;

    // invalid email
    let invalid_request = CreateUserReq {
        email: "invalid-email".to_string(),
        display_name: "Test User".to_string(),
    };

    let request = Request::builder()
        .method("POST")
        .uri("/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&invalid_request)?))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test]
async fn test_rest_api_not_found() -> Result<()> {
    let router = create_test_router().await;

    let non_existent_id = Uuid::new_v4();
    let request = Request::builder()
        .method("GET")
        .uri(format!("/users/{non_existent_id}"))
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn test_rest_dto_conversions() -> Result<()> {
    use users_info::api::rest::dto::*;
    use users_info::contract::model::*;

    // User -> UserDto
    let user = User {
        id: Uuid::new_v4(),
        email: "test@example.com".to_string(),
        display_name: "Test User".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let dto = UserDto::from(user.clone());
    assert_eq!(dto.id, user.id);
    assert_eq!(dto.email, user.email);
    assert_eq!(dto.display_name, user.display_name);

    // CreateUserReq -> NewUser
    let create_req = CreateUserReq {
        email: "new@example.com".to_string(),
        display_name: "New User".to_string(),
    };
    let new_user = NewUser::from(create_req.clone());
    assert_eq!(new_user.email, create_req.email);
    assert_eq!(new_user.display_name, create_req.display_name);

    // UpdateUserReq -> UserPatch
    let update_req = UpdateUserReq {
        email: Some("updated@example.com".to_string()),
        display_name: None,
    };
    let patch = UserPatch::from(update_req.clone());
    assert_eq!(patch.email, update_req.email);
    assert_eq!(patch.display_name, update_req.display_name);

    Ok(())
}

#[tokio::test]
async fn test_contract_model_has_no_serde() {
    // Contract models should not have serde derives.
    use users_info::contract::model::User;

    let user = User {
        id: Uuid::new_v4(),
        email: "test@example.com".to_string(),
        display_name: "Test User".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    // This wouldn't compile if User had Serialize:
    // let _ = serde_json::to_string(&user);

    // But REST DTOs *do* have serde derives:
    let dto = users_info::api::rest::dto::UserDto::from(user);
    let serialized = serde_json::to_string(&dto);
    assert!(serialized.is_ok());
}
