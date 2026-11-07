//! Checking the PIN against the argon2 hash the admin interface stored
//! (`Bun.password.hash(pin, { algorithm: "argon2id" })`, a standard PHC string).

use argon2::{Argon2, PasswordHash, PasswordVerifier};

/// 4–8 digits.
pub fn is_valid_pin(pin: &str) -> bool {
    (4..=8).contains(&pin.len()) && pin.bytes().all(|b| b.is_ascii_digit())
}

/// Does `pin` match `hash`? Slow on purpose (argon2): call from a thread.
pub fn verify(pin: &str, hash: &str) -> bool {
    if !is_valid_pin(pin) {
        return false;
    }
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(pin.as_bytes(), &parsed)
            .is_ok(),
        Err(e) => {
            log::error!("the stored PIN hash can't be read: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Made by the admin interface: `Bun.password.hash("2468", { algorithm: "argon2id" })`.
    const BUN_HASH: &str = "$argon2id$v=19$m=65536,t=2,p=1$obrux5gDNFHn+tauaT7iU9wav8SfzgEDea76FkP22kg$l70CRxpw6QiWIw0EQsf1P2p22P+D+Z5Ed9/WHQXCIzk";

    #[test]
    fn verifies_hashes_made_by_the_admin_interface() {
        assert!(verify("2468", BUN_HASH));
        assert!(!verify("2469", BUN_HASH));
        assert!(!verify("24680", BUN_HASH));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(!verify("", BUN_HASH));
        assert!(!verify("12a4", BUN_HASH));
        assert!(!verify("2468", "not a hash"));
        assert!(is_valid_pin("1234") && is_valid_pin("12345678"));
        assert!(!is_valid_pin("123") && !is_valid_pin("123456789") && !is_valid_pin("12 4"));
    }
}
