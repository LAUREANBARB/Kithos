use argon2::password_hash::phc::PasswordHash;
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use sha2::{Digest, Sha256};

pub const SESSION_TTL_SECONDS: i64 = 60 * 60 * 24 * 30;

pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).expect("operating system random source");
    let hash = Argon2::default().hash_password_with_salt(password.as_bytes(), &salt)?;
    Ok(hash.to_string())
}

pub fn verify_password(password: &str, stored: &str) -> bool {
    let Ok(parsed) = stored.parse::<PasswordHash>() else {
        return false;
    };
    Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok()
}

pub fn new_session_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("operating system random source");
    to_hex(&bytes)
}

pub fn hash_token(token: &str) -> String {
    to_hex(&Sha256::digest(token.as_bytes()))
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}
