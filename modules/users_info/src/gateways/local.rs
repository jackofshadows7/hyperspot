use async_trait::async_trait;
use std::sync::Arc;
use uuid::Uuid;

use crate::contract::{
    client::UsersInfoApi,
    error::UsersInfoError,
    model::{NewUser, User, UserPatch},
};
use crate::domain::{error::DomainError, service::Service};

/// Local implementation of the UsersInfoApi trait that delegates to the domain service
pub struct UsersInfoLocalClient {
    service: Arc<Service>,
}

impl UsersInfoLocalClient {
    pub fn new(service: Arc<Service>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl UsersInfoApi for UsersInfoLocalClient {
    async fn get_user(&self, id: Uuid) -> anyhow::Result<User> {
        self.service
            .get_user(id)
            .await
            .map_err(|e| map_domain_error_to_anyhow(e))
    }

    async fn list_users(
        &self,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> anyhow::Result<Vec<User>> {
        self.service
            .list_users(limit, offset)
            .await
            .map_err(|e| map_domain_error_to_anyhow(e))
    }

    async fn create_user(&self, new_user: NewUser) -> anyhow::Result<User> {
        self.service
            .create_user(new_user)
            .await
            .map_err(|e| map_domain_error_to_anyhow(e))
    }

    async fn update_user(&self, id: Uuid, patch: UserPatch) -> anyhow::Result<User> {
        self.service
            .update_user(id, patch)
            .await
            .map_err(|e| map_domain_error_to_anyhow(e))
    }

    async fn delete_user(&self, id: Uuid) -> anyhow::Result<()> {
        self.service
            .delete_user(id)
            .await
            .map_err(|e| map_domain_error_to_anyhow(e))
    }
}

/// Map domain errors to contract errors wrapped in anyhow
fn map_domain_error_to_anyhow(domain_error: DomainError) -> anyhow::Error {
    let contract_error = match domain_error {
        DomainError::UserNotFound { id } => UsersInfoError::not_found(id),
        DomainError::EmailAlreadyExists { email } => UsersInfoError::conflict(email),
        DomainError::InvalidEmail { email } => {
            UsersInfoError::validation(format!("Invalid email: {}", email))
        }
        DomainError::EmptyDisplayName => {
            UsersInfoError::validation("Display name cannot be empty".to_string())
        }
        DomainError::DisplayNameTooLong { len, max } => UsersInfoError::validation(format!(
            "Display name too long: {} characters (max: {})",
            len, max
        )),
        DomainError::Validation { field, message } => {
            UsersInfoError::validation(format!("{}: {}", field, message))
        }
        DomainError::Database { .. } => UsersInfoError::internal(),
    };

    anyhow::Error::new(contract_error)
}
