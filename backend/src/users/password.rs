//! Hashing and checking passwords.
//!
//! Argon2id, through the RustCrypto `argon2` crate, at its default parameters.
//! What gets stored is a PHC string — `$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>` —
//! which carries the algorithm, the version, the cost parameters and the salt
//! along with the digest. That is the property worth having: raising the cost
//! later does not invalidate a single stored hash, because every hash says how
//! it was made.
//!
//! ## Two things here are about time rather than correctness
//!
//! Hashing a password is *deliberately* slow — tens of milliseconds — which is
//! the whole point of a password hash and a disaster on an async executor. Every
//! call from a request handler goes through [`hash_in_background`] or
//! [`verify_in_background`], which move it to `spawn_blocking`. A `verify` on
//! the executor thread would stall every other request in the process for as
//! long as it ran.
//!
//! And a login attempt for an account that does not exist has to cost the same
//! as one for an account that does, or the difference tells an anonymous caller
//! which usernames are real. [`verify_absent`] spends that time against a
//! throwaway hash.

use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use rand_core::OsRng;
use thiserror::Error;

/// Shortest password accepted.
///
/// A floor rather than a policy. Composition rules ("one digit, one symbol")
/// push people towards `Password1!` and are not worth the annoyance; length is
/// the part that actually helps.
pub const MIN_PASSWORD_LEN: usize = 8;

/// Longest password accepted.
///
/// Argon2 does not need a limit; the request handler does. Hashing is priced in
/// milliseconds and the input is attacker-controlled, so an unbounded one is a
/// way to spend the server's CPU from outside.
pub const MAX_PASSWORD_LEN: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PasswordError {
    #[error("a password must be at least {MIN_PASSWORD_LEN} characters")]
    TooShort,

    #[error("a password must be at most {MAX_PASSWORD_LEN} bytes")]
    TooLong,
}

impl PasswordError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::TooShort => "password_too_short",
            Self::TooLong => "password_too_long",
        }
    }
}

/// Check a password against the only two rules there are.
pub fn check(password: &str) -> Result<(), PasswordError> {
    // Characters, not bytes: an eight-character password of non-ASCII is not
    // short, and counting its bytes would say it was.
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(PasswordError::TooShort);
    }
    if password.len() > MAX_PASSWORD_LEN {
        return Err(PasswordError::TooLong);
    }
    Ok(())
}

/// Hash a password for storage.
///
/// Blocking, and slow on purpose: see the module docs. Call
/// [`hash_in_background`] from anything async.
pub fn hash(password: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);

    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        // The only documented failures are an unsupported algorithm and an
        // output length outside the allowed range, neither of which is reachable
        // with the default parameters and a generated salt.
        .expect("argon2 with default parameters and a generated salt")
        .to_string()
}

/// Whether `password` is the one `stored` was made from.
///
/// A hash that will not parse — a file edited by hand into nonsense — verifies
/// nothing rather than raising, because the caller's next move is identical
/// either way and a malformed hash must never be a way in.
pub fn verify(password: &str, stored: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(stored) else {
        tracing::warn!("stored password hash could not be parsed; refusing the attempt");
        return false;
    };

    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// [`hash`], off the executor.
pub async fn hash_in_background(password: String) -> String {
    tokio::task::spawn_blocking(move || hash(&password))
        .await
        .expect("hashing a password does not panic")
}

/// [`verify`], off the executor.
pub async fn verify_in_background(password: String, stored: String) -> bool {
    tokio::task::spawn_blocking(move || verify(&password, &stored))
        .await
        .expect("verifying a password does not panic")
}

/// Spend a verification's worth of time on an account that does not exist.
///
/// Without this, a login for an unknown username returns in microseconds and one
/// for a known username takes as long as Argon2 does — which hands an anonymous
/// caller a list of the accounts on the instance, one guess at a time. The
/// answer is still "no"; it just costs the same either way.
pub async fn verify_absent(password: String) {
    tokio::task::spawn_blocking(move || {
        // Hashing rather than verifying against a fixed dummy: both cost one
        // Argon2 run at the same parameters, and this needs no constant checked
        // in beside it that could drift out of step with the real ones.
        let _ = hash(&password);
    })
    .await
    .expect("hashing a password does not panic");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_verifies_against_its_own_hash_and_nothing_else() {
        let stored = hash("correct horse battery staple");

        assert!(verify("correct horse battery staple", &stored));
        assert!(!verify("correct horse battery stapler", &stored));
        assert!(!verify("", &stored));
    }

    /// The salt is generated per hash, so the same password twice is two
    /// different strings — which is what stops equal hashes revealing equal
    /// passwords across accounts.
    #[test]
    fn the_same_password_hashes_differently_every_time() {
        let first = hash("correct horse battery staple");
        let second = hash("correct horse battery staple");

        assert_ne!(first, second);
        assert!(verify("correct horse battery staple", &first));
        assert!(verify("correct horse battery staple", &second));
    }

    /// What is stored has to say how it was made, or raising the cost later
    /// invalidates every existing account.
    #[test]
    fn the_stored_form_is_a_phc_string_naming_its_parameters() {
        let stored = hash("correct horse battery staple");

        assert!(stored.starts_with("$argon2id$"), "{stored}");
        assert!(stored.contains("$v="), "{stored}");
        assert!(stored.contains("m="), "{stored}");
    }

    /// A file edited by hand into nonsense must be a refusal, not a panic and
    /// certainly not a pass.
    #[test]
    fn a_hash_that_will_not_parse_verifies_nothing() {
        for stored in ["", "not a hash", "$argon2id$", "hunter2"] {
            assert!(!verify("hunter2", stored), "{stored:?} let something in");
        }
    }

    #[test]
    fn length_is_the_only_rule() {
        assert_eq!(check("short"), Err(PasswordError::TooShort));
        assert_eq!(
            check(&"a".repeat(MAX_PASSWORD_LEN + 1)),
            Err(PasswordError::TooLong)
        );

        assert!(check("longenough").is_ok());
        // No composition rules: a passphrase is not worse for being all letters.
        assert!(check("correct horse battery staple").is_ok());
    }

    /// Counted in characters. Eight non-ASCII characters is not a short
    /// password, and counting bytes would refuse it.
    #[test]
    fn length_is_counted_in_characters_rather_than_bytes() {
        assert!(check("パスワードです八").is_ok());
        assert_eq!(check("パスワード"), Err(PasswordError::TooShort));
    }
}
