use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

pub const USERNAME_MIN: usize = 3;
pub const USERNAME_MAX: usize = 32;
pub const PASSWORD_MIN: usize = 8;
pub const DISPLAY_NAME_MAX: usize = 48;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub bio: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub display_name: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    pub token: String,
    pub user: User,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsernameError {
    TooShort,
    TooLong,
    BadCharacter,
}

pub fn validate_username(username: &str) -> Result<(), UsernameError> {
    if username.chars().count() < USERNAME_MIN {
        return Err(UsernameError::TooShort);
    }
    if username.chars().count() > USERNAME_MAX {
        return Err(UsernameError::TooLong);
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(UsernameError::BadCharacter);
    }
    Ok(())
}

pub fn normalize_username(username: &str) -> String {
    username.trim().to_lowercase()
}
