use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(unique)]
    pub email: String,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

/// Data for creating a new user entity
pub struct NewUserEntity {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Data for updating an existing user entity
pub struct UpdateUserEntity {
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// Find a user by ID
pub async fn find_by_id(db: &DatabaseConnection, id: Uuid) -> Result<Option<Model>, DbErr> {
    Entity::find_by_id(id).one(db).await
}

/// Find users with pagination
pub async fn find_paginated(
    db: &DatabaseConnection,
    limit: u32,
    offset: u32,
) -> Result<Vec<Model>, DbErr> {
    Entity::find()
        .order_by_asc(Column::CreatedAt)
        .limit(limit as u64)
        .offset(offset as u64)
        .all(db)
        .await
}

/// Check if an email already exists
pub async fn email_exists(db: &DatabaseConnection, email: &str) -> Result<bool, DbErr> {
    let count = Entity::find()
        .filter(Column::Email.eq(email))
        .count(db)
        .await?;
    Ok(count > 0)
}

/// Create a new user
pub async fn create(db: &DatabaseConnection, new_user: NewUserEntity) -> Result<Model, DbErr> {
    let active_model = ActiveModel {
        id: Set(new_user.id),
        email: Set(new_user.email),
        display_name: Set(new_user.display_name),
        created_at: Set(new_user.created_at),
        updated_at: Set(new_user.updated_at),
    };

    active_model.insert(db).await
}

/// Update an existing user
pub async fn update(
    db: &DatabaseConnection,
    id: Uuid,
    update_data: UpdateUserEntity,
) -> Result<Model, DbErr> {
    let mut active_model = ActiveModel {
        id: Set(id),
        ..Default::default()
    };

    if let Some(email) = update_data.email {
        active_model.email = Set(email);
    }
    if let Some(display_name) = update_data.display_name {
        active_model.display_name = Set(display_name);
    }
    if let Some(updated_at) = update_data.updated_at {
        active_model.updated_at = Set(updated_at);
    }

    active_model.update(db).await
}

/// Delete a user by ID, returns true if a user was deleted
pub async fn delete(db: &DatabaseConnection, id: Uuid) -> Result<bool, DbErr> {
    let result = Entity::delete_by_id(id).exec(db).await?;
    Ok(result.rows_affected > 0)
}
