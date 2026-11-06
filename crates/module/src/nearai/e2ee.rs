// Derived from MultiAgency/private-ai@dd469a5 app/src/e2ee.rs. Local changes: module documentation in mod.rs.
//! End-to-end encryption to an attested model key, NEAR AI Cloud's E2EE v2:
//! X25519 ECDH, HKDF-SHA256 and XChaCha20-Poly1305. Each encrypted field is hex
//! of [ephemeral public key (32)][nonce (24)][ciphertext + tag]. Port of
//! `core/e2ee.mjs`.
use anyhow::{anyhow, Context, Result};
use chacha20poly1305::{aead::Aead, KeyInit, XChaCha20Poly1305, XNonce};
use ed25519_dalek::{SigningKey, VerifyingKey};
use hkdf::Hkdf;
use serde_json::{json, Value};
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};

const INFO: &[u8] = b"ed25519_encryption";

fn symmetric_key(shared: &[u8; 32]) -> [u8; 32] {
    let mut key = [0u8; 32];
    Hkdf::<Sha256>::new(None, shared)
        .expand(INFO, &mut key)
        .expect("32 bytes");
    key
}

fn seal(plaintext: &str, recipient: &PublicKey) -> Result<String> {
    let ephemeral = StaticSecret::from(discovery_protocol::random_bytes::<32>()?);
    let nonce = discovery_protocol::random_bytes::<24>()?;
    let shared = ephemeral.diffie_hellman(recipient);
    if !shared.was_contributory() {
        return Err(anyhow!("weak encryption public key"));
    }
    let cipher = XChaCha20Poly1305::new(&symmetric_key(shared.as_bytes()).into());
    let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), plaintext.as_bytes())
        .expect("encrypt");
    let mut out = PublicKey::from(&ephemeral).as_bytes().to_vec();
    out.extend(nonce);
    out.extend(ciphertext);
    Ok(hex::encode(out))
}

fn open(hex: &str, secret: &StaticSecret) -> Result<String> {
    let data = crate::nearai::bytes(hex)?;
    if data.len() < 56 {
        return Err(anyhow!("ciphertext too short"));
    }
    let ephemeral = PublicKey::from(<[u8; 32]>::try_from(&data[..32])?);
    let shared = secret.diffie_hellman(&ephemeral);
    if !shared.was_contributory() {
        return Err(anyhow!("weak encryption public key"));
    }
    let cipher = XChaCha20Poly1305::new(&symmetric_key(shared.as_bytes()).into());
    let plain = cipher
        .decrypt(XNonce::from_slice(&data[32..56]), &data[56..])
        .map_err(|_| anyhow!("could not decrypt a reply field"))?;
    String::from_utf8(plain).context("decrypted field is not UTF-8")
}

/// The X25519 key an Ed25519 public key encrypts to.
fn montgomery(ed25519_public_hex: &str) -> Result<PublicKey> {
    let key = VerifyingKey::from_bytes(
        &crate::nearai::bytes(ed25519_public_hex)?
            .try_into()
            .map_err(|_| anyhow!("model key is not 32 bytes"))?,
    )?;
    if key.is_weak() {
        return Err(anyhow!("weak model signing key"));
    }
    Ok(PublicKey::from(key.to_montgomery().to_bytes()))
}

/// What a streamed reply carries, decrypted.
pub struct StreamReply {
    pub id: Option<String>,
    pub content: String,
}

/// One exchange's keys: a fresh client key pair, and the model's verified Ed25519 key.
pub struct Session {
    client: SigningKey,
    client_x25519: StaticSecret,
    model_x25519: PublicKey,
    model_public_key: String,
}

impl Session {
    pub fn new(model_public_key_hex: &str) -> Result<Self> {
        let client = SigningKey::from_bytes(&discovery_protocol::random_bytes::<32>()?);
        Ok(Self {
            client_x25519: StaticSecret::from(client.to_scalar_bytes()),
            client,
            model_x25519: montgomery(model_public_key_hex)?,
            model_public_key: model_public_key_hex.to_string(),
        })
    }

    pub fn model_public_key(&self) -> &str {
        &self.model_public_key
    }

    pub fn headers(&self) -> Vec<(&'static str, String)> {
        vec![
            ("x-signing-algo", "ed25519".into()),
            (
                "x-client-pub-key",
                hex::encode(self.client.verifying_key().as_bytes()),
            ),
            ("x-model-pub-key", self.model_public_key.clone()),
            ("x-encryption-version", "2".into()),
            ("x-encrypt-all-fields", "true".into()),
        ]
    }

    /// Only text messages are supported. Unknown content shapes must never
    /// fall through to the gateway as plaintext.
    pub fn encrypt_request(&self, messages: &[Value]) -> Result<Vec<Value>> {
        messages
            .iter()
            .map(|message| {
                let content = message["content"]
                    .as_str()
                    .ok_or_else(|| anyhow!("only text messages are supported"))?;
                let role = message["role"]
                    .as_str()
                    .ok_or_else(|| anyhow!("message has no role"))?;
                Ok(json!({"role": role, "content": seal(content, &self.model_x25519)?}))
            })
            .collect()
    }

    /// Open and join answer fragments. The survey does not consume reasoning
    /// fields; the signed-turn checker still authenticates the complete stream.
    pub fn decrypt_stream(&self, events: &[Value]) -> Result<StreamReply> {
        let mut content = String::new();
        for event in events {
            let Some(choice) = event["choices"].get(0) else {
                continue;
            };
            let delta = &choice["delta"];
            if let Some(fragment) = delta["content"].as_str().filter(|s| !s.is_empty()) {
                content.push_str(&open(fragment, &self.client_x25519)?);
            }
            if delta
                .get("tool_calls")
                .is_some_and(|calls| !calls.is_null())
            {
                return Err(anyhow!("tool calls are not supported"));
            }
        }
        Ok(StreamReply {
            id: events
                .first()
                .and_then(|e| e["id"].as_str())
                .map(String::from),
            content,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_order_keys_are_rejected_before_encryption_or_decryption() {
        assert!(Session::new(&format!("01{}", "00".repeat(31))).is_err());
        assert!(seal("private", &PublicKey::from([0; 32])).is_err());
        assert!(open(&hex::encode([0; 72]), &StaticSecret::from([7; 32])).is_err());
    }

    #[test]
    fn text_round_trip_works_and_nontext_content_is_never_sent_plain() {
        let model = SigningKey::from_bytes(&[7; 32]);
        let session = Session::new(&hex::encode(model.verifying_key().as_bytes())).unwrap();
        let messages = session
            .encrypt_request(&[json!({"role":"user", "content":"private text"})])
            .unwrap();
        assert_ne!(messages[0]["content"], "private text");
        let secret = StaticSecret::from(model.to_scalar_bytes());
        assert_eq!(
            open(messages[0]["content"].as_str().unwrap(), &secret).unwrap(),
            "private text"
        );
        let client = PublicKey::from(&session.client_x25519);
        let reply = session
            .decrypt_stream(&[
                json!({"id":"chat", "choices":[{"delta":{
                    "content":seal("first ", &client).unwrap(),
                    "reasoning_content":"unused reasoning is not decrypted"
                }}]}),
                json!({"choices":[{"delta":{"content":seal("second", &client).unwrap()}}]}),
            ])
            .unwrap();
        assert_eq!(reply.id.as_deref(), Some("chat"));
        assert_eq!(reply.content, "first second");
        assert!(session
            .encrypt_request(&[
                json!({"role":"user", "content":[{"type":"text", "text":"private"}]})
            ])
            .is_err());
    }
}
