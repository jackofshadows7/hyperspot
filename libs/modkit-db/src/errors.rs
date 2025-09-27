//! Shared database error helpers (SQLSTATE categorization, etc.)

/// Returns true if the given SQLSTATE code represents a unique constraint violation
/// across popular backends (Postgres 23505, SQLite 2067, MySQL 1062).
pub fn is_unique_violation_code(code: &str) -> bool {
    matches!(code, "23505" | "2067" | "1062")
}

pub fn is_sqlx_unique_violation(db: &dyn sqlx::error::DatabaseError) -> bool {
    db.code().map(|c| is_unique_violation_code(c.as_ref())).unwrap_or(false)
}

#[cfg(feature = "sea-orm")]
pub fn is_seaorm_unique_violation(err: &sea_orm::RuntimeErr) -> bool {
    // Check the error message for unique constraint violations
    let msg = err.to_string().to_lowercase();
    msg.contains("unique") || msg.contains("duplicate") || msg.contains("constraint")
}
