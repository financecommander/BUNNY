use ml_kem::kem::{Decapsulate, Encapsulate};
use ml_kem::{EncodedSizeUser, KemCore, MlKem768};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use x25519_dalek::{EphemeralSecret, PublicKey as X25519PublicKey};

use crate::cipher::CipherSuite;
use crate::error::{CryptoError, Result};
use crate::kdf::derive_session_key;
use crate::types::SymmetricKey;

/// Public keys sent over the wire during key exchange.
#[derive(Clone, Serialize, Deserialize)]
pub struct KeyExchangePublicBundle {
    pub x25519_public: [u8; 32],
    pub mlkem_ek: Vec<u8>,
    pub preferred_suite: CipherSuite,
}

/// Responder's reply containing encapsulated keys.
#[derive(Clone, Serialize, Deserialize)]
pub struct KeyExchangeResponse {
    pub x25519_public: [u8; 32],
    pub mlkem_ciphertext: Vec<u8>,
    pub agreed_suite: CipherSuite,
}

/// Initiator's side of the hybrid key exchange.
pub struct KeyExchangeInitiator {
    x25519_secret: EphemeralSecret,
    x25519_public: X25519PublicKey,
    mlkem_dk: <MlKem768 as KemCore>::DecapsulationKey,
    mlkem_ek: <MlKem768 as KemCore>::EncapsulationKey,
}

impl KeyExchangeInitiator {
    /// Generate fresh ephemeral keys for a new session.
    pub fn new() -> Self {
        let x25519_secret = EphemeralSecret::random_from_rng(OsRng);
        let x25519_public = X25519PublicKey::from(&x25519_secret);

        let (mlkem_dk, mlkem_ek) = MlKem768::generate(&mut OsRng);

        Self {
            x25519_secret,
            x25519_public,
            mlkem_dk,
            mlkem_ek,
        }
    }

    /// Export public keys to send to the responder.
    pub fn public_bundle(&self) -> KeyExchangePublicBundle {
        KeyExchangePublicBundle {
            x25519_public: self.x25519_public.to_bytes(),
            mlkem_ek: self.mlkem_ek.as_bytes().to_vec(),
            preferred_suite: CipherSuite::auto_detect(),
        }
    }

    /// Complete the exchange after receiving the responder's reply.
    /// Returns the derived session key and agreed cipher suite.
    pub fn complete(self, response: &KeyExchangeResponse) -> Result<(SymmetricKey, CipherSuite)> {
        // X25519 DH
        let peer_x25519 = X25519PublicKey::from(response.x25519_public);
        let shared_classical = self.x25519_secret.diffie_hellman(&peer_x25519);

        // ML-KEM decapsulate
        let mlkem_ct_array = hybrid_array::Array::try_from(response.mlkem_ciphertext.as_slice())
            .map_err(|_| CryptoError::KemDecapsulationFailed)?;
        let shared_pq = self
            .mlkem_dk
            .decapsulate(&mlkem_ct_array)
            .map_err(|_| CryptoError::KemDecapsulationFailed)?;

        // Combine both shared secrets
        let mut ikm = Vec::with_capacity(32 + 32);
        ikm.extend_from_slice(shared_classical.as_bytes());
        ikm.extend_from_slice(shared_pq.as_ref());

        let session_key = derive_session_key(&ikm, b"bunny-session-key")?;

        Ok((session_key, response.agreed_suite))
    }
}

/// Responder side of the key exchange.
pub struct KeyExchangeResponder;

impl KeyExchangeResponder {
    /// Process an initiator's public bundle and produce a response + session key.
    pub fn respond(
        bundle: &KeyExchangePublicBundle,
    ) -> Result<(KeyExchangeResponse, SymmetricKey, CipherSuite)> {
        // Generate ephemeral X25519 keypair
        let x25519_secret = EphemeralSecret::random_from_rng(OsRng);
        let x25519_public = X25519PublicKey::from(&x25519_secret);

        // X25519 DH
        let peer_x25519 = X25519PublicKey::from(bundle.x25519_public);
        let shared_classical = x25519_secret.diffie_hellman(&peer_x25519);

        // ML-KEM encapsulate: reconstruct EncapsulationKey from bytes
        let ek_array = hybrid_array::Array::try_from(bundle.mlkem_ek.as_slice())
            .map_err(|_| CryptoError::KemEncapsulationFailed)?;
        let mlkem_ek =
            <MlKem768 as KemCore>::EncapsulationKey::from_bytes(&ek_array);
        let (mlkem_ct, shared_pq) = mlkem_ek
            .encapsulate(&mut OsRng)
            .map_err(|_| CryptoError::KemEncapsulationFailed)?;

        // Combine both shared secrets
        let mut ikm = Vec::with_capacity(32 + 32);
        ikm.extend_from_slice(shared_classical.as_bytes());
        ikm.extend_from_slice(shared_pq.as_ref());

        let session_key = derive_session_key(&ikm, b"bunny-session-key")?;
        let agreed_suite = bundle.preferred_suite;

        let response = KeyExchangeResponse {
            x25519_public: x25519_public.to_bytes(),
            mlkem_ciphertext: mlkem_ct.as_slice().to_vec(),
            agreed_suite,
        };

        Ok((response, session_key, agreed_suite))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_exchange_roundtrip() {
        let initiator = KeyExchangeInitiator::new();
        let bundle = initiator.public_bundle();

        let (response, responder_key, suite) = KeyExchangeResponder::respond(&bundle).unwrap();
        let (initiator_key, initiator_suite) = initiator.complete(&response).unwrap();

        // Both sides derive the same key
        assert_eq!(initiator_key.as_bytes(), responder_key.as_bytes());
        assert_eq!(suite, initiator_suite);
    }

    #[test]
    fn different_sessions_produce_different_keys() {
        let init1 = KeyExchangeInitiator::new();
        let bundle1 = init1.public_bundle();
        let (resp1, key1, _) = KeyExchangeResponder::respond(&bundle1).unwrap();
        let (ikey1, _) = init1.complete(&resp1).unwrap();

        let init2 = KeyExchangeInitiator::new();
        let bundle2 = init2.public_bundle();
        let (resp2, key2, _) = KeyExchangeResponder::respond(&bundle2).unwrap();
        let (ikey2, _) = init2.complete(&resp2).unwrap();

        assert_ne!(key1.as_bytes(), key2.as_bytes());
        assert_ne!(ikey1.as_bytes(), ikey2.as_bytes());
    }

    #[test]
    fn bundle_serialization_roundtrip() {
        let initiator = KeyExchangeInitiator::new();
        let bundle = initiator.public_bundle();

        let json = serde_json::to_string(&bundle).unwrap();
        let deserialized: KeyExchangePublicBundle = serde_json::from_str(&json).unwrap();

        assert_eq!(bundle.x25519_public, deserialized.x25519_public);
        assert_eq!(bundle.mlkem_ek, deserialized.mlkem_ek);
    }
}
