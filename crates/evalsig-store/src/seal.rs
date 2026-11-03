//! Sealing with age to the referee's X25519 recipient (D-013). Sealed files are age
//! v1 files that `age -d -i identity.txt` and `rage -d -i identity.txt` read.

use age::secrecy::ExposeSecret;

#[derive(Debug, thiserror::Error)]
pub enum SealError {
    #[error("not an age X25519 identity")]
    Identity,
    #[error("not an age X25519 recipient")]
    Recipient,
    #[error("encryption failed: {0}")]
    Encrypt(String),
    #[error("the sealed file cannot be opened with this identity: {0}")]
    Decrypt(String),
}

/// A new identity, as its secret key line (`AGE-SECRET-KEY-1…`) and recipient
/// (`age1…`).
pub fn generate() -> (String, String) {
    let id = age::x25519::Identity::generate();
    (
        id.to_string().expose_secret().to_owned(),
        id.to_public().to_string(),
    )
}

/// 32 random bytes as hex, for a bearer token: the BLAKE3 hash of a fresh X25519
/// secret, which age draws from the operating system's generator.
pub fn random_token() -> String {
    let id = age::x25519::Identity::generate();
    blake3::hash(id.to_string().expose_secret().as_bytes())
        .to_hex()
        .to_string()
}

/// The secret key line of an identity file: the first line that is not a comment.
fn identity_line(text: &str) -> Option<&str> {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
}

/// The recipient of an identity file's key.
pub fn recipient_of(identity_file: &str) -> Result<String, SealError> {
    let id: age::x25519::Identity = identity_line(identity_file)
        .ok_or(SealError::Identity)?
        .parse()
        .map_err(|_| SealError::Identity)?;
    Ok(id.to_public().to_string())
}

/// Encrypt to a recipient (`age1…`).
pub fn seal(recipient: &str, plaintext: &[u8]) -> Result<Vec<u8>, SealError> {
    let r: age::x25519::Recipient = recipient.trim().parse().map_err(|_| SealError::Recipient)?;
    age::encrypt(&r, plaintext).map_err(|e| SealError::Encrypt(e.to_string()))
}

/// Decrypt with an identity file's key.
pub fn open(identity_file: &str, ciphertext: &[u8]) -> Result<Vec<u8>, SealError> {
    let id: age::x25519::Identity = identity_line(identity_file)
        .ok_or(SealError::Identity)?
        .parse()
        .map_err(|_| SealError::Identity)?;
    age::decrypt(&id, ciphertext).map_err(|e| SealError::Decrypt(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_bytes_open_with_the_identity_only() {
        let (secret, recipient) = generate();
        let file = format!("# created: test\n# public key: {recipient}\n{secret}\n");
        assert_eq!(recipient_of(&file).unwrap(), recipient);
        let sealed = seal(&recipient, b"labels").unwrap();
        assert!(sealed.starts_with(b"age-encryption.org/v1\n"));
        assert_eq!(open(&file, &sealed).unwrap(), b"labels");
        let (other, _) = generate();
        assert!(matches!(open(&other, &sealed), Err(SealError::Decrypt(_))));
    }
}
