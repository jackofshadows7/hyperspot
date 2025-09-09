//! SeaORM-backed repository implementation for the domain port.
//!
//! This struct is generic over `C: ConnectionTrait`, so you can construct it
//! with a `DatabaseConnection` **or** a transactional connection.
//! For transactional flows create a new instance with a transaction connection
//! and pass it down to a short-lived service (or expose lower-level APIs).

use anyhow::Context;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, Set,
};
use uuid::Uuid;

use crate::contract::User;
use crate::domain::repo::UsersRepository;
use crate::infra::storage::entity::{ActiveModel as UserAM, Column, Entity as UserEntity};

/// SeaORM repository impl.
/// Holds a connection object; its lifetime/ownership is up to the caller.
pub struct SeaOrmUsersRepository<C>
where
    C: ConnectionTrait + Send + Sync,
{
    conn: C,
}

impl<C> SeaOrmUsersRepository<C>
where
    C: ConnectionTrait + Send + Sync,
{
    pub fn new(conn: C) -> Self {
        Self { conn }
    }
}

#[async_trait::async_trait]
impl<C> UsersRepository for SeaOrmUsersRepository<C>
where
    C: ConnectionTrait + Send + Sync + 'static,
{
    async fn find_by_id(&self, id: Uuid) -> anyhow::Result<Option<User>> {
        let found = UserEntity::find_by_id(id)
            .one(&self.conn)
            .await
            .context("find_by_id failed")?;
        Ok(found.map(Into::into))
    }

    async fn email_exists(&self, email: &str) -> anyhow::Result<bool> {
        let count = UserEntity::find()
            .filter(Column::Email.eq(email))
            .count(&self.conn)
            .await
            .context("email_exists failed")?;
        Ok(count > 0)
    }

    async fn insert(&self, u: User) -> anyhow::Result<()> {
        let m = UserAM {
            id: Set(u.id),
            email: Set(u.email),
            display_name: Set(u.display_name),
            created_at: Set(u.created_at),
            updated_at: Set(u.updated_at),
        };
        let _ = m.insert(&self.conn).await.context("insert failed")?;
        Ok(())
    }

    async fn update(&self, u: User) -> anyhow::Result<()> {
        // Minimal upsert-by-PK via ActiveModel::update
        let m = UserAM {
            id: Set(u.id),
            email: Set(u.email),
            display_name: Set(u.display_name),
            created_at: Set(u.created_at),
            updated_at: Set(u.updated_at),
        };
        let _ = m.update(&self.conn).await.context("update failed")?;
        Ok(())
    }

    async fn delete(&self, id: Uuid) -> anyhow::Result<bool> {
        let res = UserEntity::delete_by_id(id)
            .exec(&self.conn)
            .await
            .context("delete failed")?;
        Ok(res.rows_affected > 0)
    }

    async fn list_paginated(&self, limit: u32, offset: u32) -> anyhow::Result<Vec<User>> {
        let rows = UserEntity::find()
            .order_by_asc(Column::CreatedAt)
            .limit(limit as u64)
            .offset(offset as u64)
            .all(&self.conn)
            .await
            .context("list_paginated failed")?;
        Ok(rows.into_iter().map(Into::into).collect())
    }
}
