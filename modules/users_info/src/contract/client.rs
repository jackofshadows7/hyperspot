use async_trait::async_trait;
use uuid::Uuid;

use crate::contract::model::{NewUser, User, UserPatch};

/// Public API trait for the users_info module that other modules can use
#[async_trait]
pub trait UsersInfoApi: Send + Sync {
    /// Get a user by ID
    async fn get_user(&self, id: Uuid) -> anyhow::Result<User>;
    
    /// List users with optional pagination
    async fn list_users(&self, limit: Option<u32>, offset: Option<u32>) -> anyhow::Result<Vec<User>>;
    
    /// Create a new user
    async fn create_user(&self, new_user: NewUser) -> anyhow::Result<User>;
    
    /// Update a user with partial data
    async fn update_user(&self, id: Uuid, patch: UserPatch) -> anyhow::Result<User>;
    
    /// Delete a user by ID
    async fn delete_user(&self, id: Uuid) -> anyhow::Result<()>;
}
