//! Authenticated encryption for persisted repository OAuth credentials.
use factory_core::identity::{
    EncryptedRepositoryToken, RepositoryAccessError, RepositoryAccessToken, RepositoryTokenCipher,
};
use ring::{
    aead::{self, Aad, LessSafeKey, Nonce, UnboundKey},
    rand::{SecureRandom, SystemRandom},
};
use uuid::Uuid;

const RENEWABLE_PREFIX: &[u8] = b"\0factory-token-v2\0";
const CIPHERTEXT_LIMIT: usize = 8192;

pub struct RepositoryAeadCipher {
    key_id: String,
    key: LessSafeKey,
}

impl RepositoryAeadCipher {
    /// Key material is supplied by the host; never generated or logged here.
    pub fn new(key_id: String, key: &[u8]) -> Result<Self, RepositoryAccessError> {
        if key_id.is_empty() || key_id.len() > 128 || key_id.chars().any(char::is_control) {
            return Err(RepositoryAccessError::InvalidInput);
        }
        let key = UnboundKey::new(&aead::AES_256_GCM, key)
            .map_err(|_| RepositoryAccessError::Unavailable)?;
        Ok(Self {
            key_id,
            key: LessSafeKey::new(key),
        })
    }
    fn aad(&self, user_id: Uuid) -> String {
        format!(
            "factory/github/repository-token/v1/{}/{user_id}",
            self.key_id
        )
    }
}

impl RepositoryTokenCipher for RepositoryAeadCipher {
    fn seal(
        &self,
        user_id: Uuid,
        token: &RepositoryAccessToken,
    ) -> Result<EncryptedRepositoryToken, RepositoryAccessError> {
        if user_id.is_nil() {
            return Err(RepositoryAccessError::InvalidInput);
        }
        let mut nonce = [0u8; 12];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| RepositoryAccessError::Unavailable)?;
        let mut ciphertext = if let Some(renewal) = token.renewal() {
            let mut payload = RENEWABLE_PREFIX.to_vec();
            payload.extend_from_slice(&renewal.expires_at().to_be_bytes());
            payload.extend_from_slice(token.secret().as_bytes());
            payload.push(0);
            payload.extend_from_slice(renewal.secret().as_bytes());
            payload
        } else {
            token.secret().as_bytes().to_vec()
        };
        if ciphertext.len() + aead::AES_256_GCM.tag_len() > CIPHERTEXT_LIMIT {
            return Err(RepositoryAccessError::InvalidLogin);
        }
        self.key
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(self.aad(user_id).as_bytes()),
                &mut ciphertext,
            )
            .map_err(|_| RepositoryAccessError::Unavailable)?;
        Ok(EncryptedRepositoryToken {
            key_id: self.key_id.clone(),
            nonce: nonce.to_vec(),
            ciphertext,
        })
    }

    fn open(
        &self,
        user_id: Uuid,
        token: &EncryptedRepositoryToken,
    ) -> Result<RepositoryAccessToken, RepositoryAccessError> {
        if user_id.is_nil()
            || token.key_id != self.key_id
            || token.ciphertext.len() > CIPHERTEXT_LIMIT
        {
            return Err(RepositoryAccessError::Unavailable);
        }
        let nonce: [u8; 12] = token
            .nonce
            .as_slice()
            .try_into()
            .map_err(|_| RepositoryAccessError::Unavailable)?;
        let mut bytes = token.ciphertext.clone();
        let plaintext = self
            .key
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(self.aad(user_id).as_bytes()),
                &mut bytes,
            )
            .map_err(|_| RepositoryAccessError::Unavailable)?;
        decode_token(plaintext).ok_or(RepositoryAccessError::Unavailable)
    }
}

fn decode_token(plaintext: &[u8]) -> Option<RepositoryAccessToken> {
    if let Some(payload) = plaintext.strip_prefix(RENEWABLE_PREFIX) {
        let (timestamp, secrets) = payload.split_at_checked(8)?;
        let expires_at = i64::from_be_bytes(timestamp.try_into().ok()?);
        let (access, refresh) = std::str::from_utf8(secrets).ok()?.split_once('\0')?;
        RepositoryAccessToken::renewable(access.to_owned(), refresh.to_owned(), expires_at).ok()
    } else {
        RepositoryAccessToken::new(std::str::from_utf8(plaintext).ok()?.to_owned()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_cipher_preserves_renewal_without_exposing_secrets() {
        let cipher = RepositoryAeadCipher::new("test-key".into(), &[7; 32]).unwrap();
        let user = Uuid::new_v4();
        let token = RepositoryAccessToken::renewable(
            "synthetic-access".into(),
            "synthetic-refresh".into(),
            2_000_000_000,
        )
        .unwrap();
        let sealed = cipher.seal(user, &token).unwrap();
        assert!(
            !sealed
                .ciphertext
                .windows(17)
                .any(|window| window == b"synthetic-refresh")
        );
        let opened = cipher.open(user, &sealed).unwrap();
        assert_eq!(opened.secret(), "synthetic-access");
        let renewal = opened.renewal().unwrap();
        assert_eq!(renewal.secret(), "synthetic-refresh");
        assert_eq!(renewal.expires_at(), 2_000_000_000);
    }

    #[test]
    fn token_cipher_authenticates_user_key_and_ciphertext() {
        let cipher = RepositoryAeadCipher::new("test-key".into(), &[7; 32]).unwrap();
        let other = RepositoryAeadCipher::new("test-key".into(), &[8; 32]).unwrap();
        let user = Uuid::new_v4();
        let token = RepositoryAccessToken::new("synthetic-token".into()).unwrap();
        let sealed = cipher.seal(user, &token).unwrap();
        let second = cipher.seal(user, &token).unwrap();
        assert_ne!(sealed.nonce, second.nonce);
        assert_eq!(
            cipher.open(user, &sealed).unwrap().secret(),
            "synthetic-token"
        );
        assert!(cipher.open(Uuid::new_v4(), &sealed).is_err());
        assert!(other.open(user, &sealed).is_err());
        let mut changed = sealed.clone();
        changed.ciphertext[0] ^= 1;
        assert!(cipher.open(user, &changed).is_err());
        let mut changed_nonce = sealed.clone();
        changed_nonce.nonce[0] ^= 1;
        assert!(cipher.open(user, &changed_nonce).is_err());
        assert!(RepositoryAeadCipher::new("bad".into(), &[1; 16]).is_err());
    }
}
