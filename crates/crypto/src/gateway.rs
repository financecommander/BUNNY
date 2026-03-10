use tracing::{info, warn};

use crate::cipher::SwarmCipher;
use crate::envelope::SwarmEnvelope;
use crate::error::{CryptoError, Result};
use crate::key_exchange::{KeyExchangePublicBundle, KeyExchangeResponse, KeyExchangeResponder};
use crate::session::{SessionStore, SwarmSession};
use crate::types::{AgentId, SessionId, PROTOCOL_VERSION};

/// A validated, decrypted inference request ready for routing.
#[derive(Debug)]
pub struct DecryptedRequest {
    pub sender: AgentId,
    pub session_id: SessionId,
    pub payload: Vec<u8>,
    pub received_at: u64,
}

/// Result from the inference runtime to be re-encrypted.
pub struct InferenceResult {
    pub payload: Vec<u8>,
}

/// The inference gateway decrypts requests at the execution boundary,
/// validates them, routes to the inference runtime, and re-encrypts results.
///
/// Flow: encrypted swarm → decrypt → validate → route → Triton → re-encrypt → return
pub struct InferenceGateway {
    session_store: SessionStore,
    identity: AgentId,
    max_envelope_age_ms: u64,
}

impl InferenceGateway {
    pub fn new(
        identity: AgentId,
        session_store: SessionStore,
        max_envelope_age_ms: u64,
    ) -> Self {
        Self {
            session_store,
            identity,
            max_envelope_age_ms,
        }
    }

    /// Decrypt an incoming envelope at the security boundary.
    ///
    /// Validates: protocol version, session exists, envelope age, replay, AEAD auth.
    pub async fn decrypt_request(&self, envelope: &SwarmEnvelope) -> Result<DecryptedRequest> {
        // 1. Protocol version
        if envelope.version != PROTOCOL_VERSION {
            return Err(CryptoError::ProtocolVersionMismatch {
                expected: PROTOCOL_VERSION,
                actual: envelope.version,
            });
        }

        // 2. Envelope age (TTL)
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        if now.saturating_sub(envelope.timestamp_ms) > self.max_envelope_age_ms {
            warn!(
                sender = ?envelope.sender,
                age_ms = now - envelope.timestamp_ms,
                "envelope expired"
            );
            return Err(CryptoError::SessionExpired);
        }

        // 3. Session lookup + decrypt (replay protection inside session.open)
        let result = self
            .session_store
            .get_mut(&envelope.session_id, |session| {
                // 4. Session expiry
                if session.is_expired(self.session_store.max_session_age_ms) {
                    return Err(CryptoError::SessionExpired);
                }
                // 5. Decrypt + authenticate + replay check
                session.open(envelope)
            })
            .await;

        match result {
            Some(Ok(plaintext)) => {
                info!(sender = ?envelope.sender, seq = envelope.sequence, "request decrypted at gateway");
                Ok(DecryptedRequest {
                    sender: envelope.sender.clone(),
                    session_id: envelope.session_id.clone(),
                    payload: plaintext,
                    received_at: now,
                })
            }
            Some(Err(e)) => Err(e),
            None => Err(CryptoError::SessionNotFound(envelope.session_id.clone())),
        }
    }

    /// Re-encrypt an inference result for return to the requesting agent.
    pub async fn encrypt_response(
        &self,
        session_id: &SessionId,
        result: &InferenceResult,
    ) -> Result<SwarmEnvelope> {
        let identity = self.identity.clone();
        let payload = result.payload.clone();

        let envelope = self
            .session_store
            .get_mut(session_id, |session| session.seal(&identity, &payload))
            .await;

        match envelope {
            Some(Ok(env)) => {
                info!(session = ?session_id, "response encrypted at gateway");
                Ok(env)
            }
            Some(Err(e)) => Err(e),
            None => Err(CryptoError::SessionNotFound(session_id.clone())),
        }
    }

    /// Full gateway flow: decrypt → (caller runs inference) → re-encrypt.
    pub async fn process<F, Fut>(
        &self,
        envelope: &SwarmEnvelope,
        inference_fn: F,
    ) -> Result<SwarmEnvelope>
    where
        F: FnOnce(DecryptedRequest) -> Fut,
        Fut: std::future::Future<Output = std::result::Result<InferenceResult, CryptoError>>,
    {
        let request = self.decrypt_request(envelope).await?;
        let session_id = request.session_id.clone();
        let result = inference_fn(request).await?;
        self.encrypt_response(&session_id, &result).await
    }

    /// Establish a new session via hybrid key exchange.
    pub async fn establish_session(
        &self,
        peer: AgentId,
        bundle: &KeyExchangePublicBundle,
    ) -> Result<(KeyExchangeResponse, SessionId)> {
        let (response, session_key, suite) = KeyExchangeResponder::respond(bundle)?;

        let session_id = SessionId::new();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let cipher = SwarmCipher::with_suite(&session_key, suite);

        let session = SwarmSession {
            session_id: session_id.clone(),
            peer,
            cipher,
            created_at: now,
            last_activity: now,
            send_sequence: 0,
            recv_sequence: 0,
            recv_bitmap: 0,
            recv_initialized: false,
        };

        self.session_store.insert(session).await;
        info!(session = ?session_id, "new session established at gateway");

        Ok((response, session_id))
    }

    pub fn identity(&self) -> &AgentId {
        &self.identity
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cipher::CipherSuite;
    use crate::types::SymmetricKey;

    async fn setup_gateway() -> (InferenceGateway, SessionId, SwarmCipher, AgentId) {
        let gateway_id = AgentId::new();
        let store = SessionStore::new(3_600_000); // 1 hour
        let gateway = InferenceGateway::new(gateway_id.clone(), store.clone(), 60_000);

        let key = SymmetricKey::from_bytes([0x42; 32]);
        let suite = CipherSuite::Aes256Gcm;
        let session_id = SessionId::new();
        let peer = AgentId::new();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let session = SwarmSession {
            session_id: session_id.clone(),
            peer: peer.clone(),
            cipher: SwarmCipher::with_suite(&key, suite),
            created_at: now,
            last_activity: now,
            send_sequence: 0,
            recv_sequence: 0,
            recv_bitmap: 0,
            recv_initialized: false,
        };

        store.insert(session).await;

        let client_cipher = SwarmCipher::with_suite(&key, suite);
        (gateway, session_id, client_cipher, peer)
    }

    #[tokio::test]
    async fn gateway_decrypt_encrypt_roundtrip() {
        let (gateway, session_id, client_cipher, peer) = setup_gateway().await;

        // Client encrypts a request
        let request_payload = b"inference request";
        let envelope =
            SwarmEnvelope::seal(&client_cipher, &peer, &session_id, 1, request_payload).unwrap();

        // Gateway decrypts
        let decrypted = gateway.decrypt_request(&envelope).await.unwrap();
        assert_eq!(decrypted.payload, request_payload);
        assert_eq!(decrypted.sender, peer);

        // Gateway encrypts response
        let result = InferenceResult {
            payload: b"inference result".to_vec(),
        };
        let response_envelope = gateway
            .encrypt_response(&session_id, &result)
            .await
            .unwrap();

        // Client decrypts response
        let response_plaintext = response_envelope.open(&client_cipher).unwrap();
        assert_eq!(response_plaintext, b"inference result");
    }

    #[tokio::test]
    async fn gateway_process_flow() {
        let (gateway, session_id, client_cipher, peer) = setup_gateway().await;

        let envelope =
            SwarmEnvelope::seal(&client_cipher, &peer, &session_id, 1, b"model_call").unwrap();

        let response = gateway
            .process(&envelope, |req| async move {
                assert_eq!(req.payload, b"model_call");
                Ok(InferenceResult {
                    payload: b"logits_result".to_vec(),
                })
            })
            .await
            .unwrap();

        let result = response.open(&client_cipher).unwrap();
        assert_eq!(result, b"logits_result");
    }

    #[tokio::test]
    async fn gateway_rejects_unknown_session() {
        let gateway_id = AgentId::new();
        let store = SessionStore::new(3_600_000);
        let gateway = InferenceGateway::new(gateway_id, store, 60_000);

        let key = SymmetricKey::from_bytes([0x42; 32]);
        let cipher = SwarmCipher::with_suite(&key, CipherSuite::Aes256Gcm);
        let unknown_session = SessionId::new();
        let sender = AgentId::new();

        let envelope =
            SwarmEnvelope::seal(&cipher, &sender, &unknown_session, 1, b"data").unwrap();

        let err = gateway.decrypt_request(&envelope).await.unwrap_err();
        assert!(matches!(err, CryptoError::SessionNotFound(_)));
    }

    #[tokio::test]
    async fn gateway_key_exchange_establishes_session() {
        let gateway_id = AgentId::new();
        let store = SessionStore::new(3_600_000);
        let gateway = InferenceGateway::new(gateway_id, store.clone(), 60_000);

        let peer = AgentId::new();
        let initiator = crate::key_exchange::KeyExchangeInitiator::new();
        let bundle = initiator.public_bundle();

        let (response, session_id) = gateway.establish_session(peer, &bundle).await.unwrap();

        // Session should exist in the store
        assert!(store.contains(&session_id).await);

        // Initiator can complete the exchange
        let (client_key, suite) = initiator.complete(&response).unwrap();

        // Client can now encrypt and gateway can decrypt
        let client_cipher = SwarmCipher::with_suite(&client_key, suite);
        let envelope = SwarmEnvelope::seal(
            &client_cipher,
            &AgentId::new(),
            &session_id,
            1,
            b"hello gateway",
        )
        .unwrap();

        let decrypted = gateway.decrypt_request(&envelope).await.unwrap();
        assert_eq!(decrypted.payload, b"hello gateway");
    }
}
