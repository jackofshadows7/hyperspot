use crate::contract::model::User;
use crate::infra::storage::entity::Model as UserEntity;

/// Convert a database entity to a contract model
pub fn entity_to_contract(entity: UserEntity) -> User {
    User {
        id: entity.id,
        email: entity.email,
        display_name: entity.display_name,
        created_at: entity.created_at,
        updated_at: entity.updated_at,
    }
}
