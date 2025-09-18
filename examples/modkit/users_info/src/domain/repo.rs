use crate::contract::model::User;
use async_trait::async_trait;
use modkit::api::odata::ODataQuery;
use uuid::Uuid;

/// Port for the domain layer: persistence operations the domain needs.
/// Object-safe and async-friendly via `async_trait`.
#[async_trait]
pub trait UsersRepository: Send + Sync {
    /// Load a user by id.
    async fn find_by_id(&self, id: Uuid) -> anyhow::Result<Option<User>>;
    /// Check uniqueness by email.
    async fn email_exists(&self, email: &str) -> anyhow::Result<bool>;
    /// Insert a fully-formed domain user.
    ///
    /// Service computes id/timestamps/validation; repo persists.
    async fn insert(&self, u: User) -> anyhow::Result<()>;
    /// Update an existing user (by primary key in `u.id`).
    async fn update(&self, u: User) -> anyhow::Result<()>;
    /// Delete by id. Returns true if a row was deleted.
    async fn delete(&self, id: Uuid) -> anyhow::Result<bool>;
    /// List with simple pagination.
    async fn list_paginated(
        &self,
        od_query: ODataQuery,
        limit: u64,  // TODO: will be moved to OData filter
        offset: u64, // TODO: will be moved to OData filter
    ) -> anyhow::Result<Vec<User>>;
}
