// Derived from MultiAgency/private-ai@dd469a5 app/src/e2ee.rs. Local changes: module documentation in mod.rs.
//! End-to-end encryption to an attested model key, NEAR AI Cloud's E2EE v3:
//! X25519 ECDH, HKDF-SHA256 and XChaCha20-Poly1305. Each encrypted field is hex
//! of [ephemeral public key (32)][nonce (24)][ciphertext + tag]. Requests bind
//! the reply public key and protocol field label as AEAD associated data;
//! v3 replies retain the v2 wire format. There is no fallback to unbound v2.
//! See nearai/inference-proxy#321 and nearai/inference-sdk#127.
use anyhow::{anyhow, Context, Result};
use chacha20poly1305::{
    aead::{Aead, Payload},
    KeyInit, XChaCha20Poly1305, XNonce,
};
use ed25519_dalek::{SigningKey, VerifyingKey};
use hkdf::Hkdf;
use serde_json::{json, Value};
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};

const INFO: &[u8] = b"ed25519_encryption";
const CONTENT_FIELD: &str = "messages[].content";

/// V3's canonical Ed25519 suite and field binding, as specified by the model
/// proxy. The reply key is the exact raw key also sent in x-client-pub-key.
fn request_aad(reply_public: &[u8; 32], field: &str) -> Vec<u8> {
    let mut aad = b"nearai-e2ee-request-v3".to_vec();
    aad.push(2); // Ed25519 suite
    aad.extend(reply_public);
    aad.push(0);
    aad.extend(field.as_bytes());
    aad
}

fn symmetric_key(shared: &[u8; 32]) -> [u8; 32] {
    let mut key = [0u8; 32];
    Hkdf::<Sha256>::new(None, shared)
        .expand(INFO, &mut key)
        .expect("32 bytes");
    key
}

fn seal(plaintext: &str, recipient: &PublicKey, aad: &[u8]) -> Result<String> {
    let ephemeral = StaticSecret::from(discovery_protocol::random_bytes::<32>()?);
    let nonce = discovery_protocol::random_bytes::<24>()?;
    let shared = ephemeral.diffie_hellman(recipient);
    if !shared.was_contributory() {
        return Err(anyhow!("weak encryption public key"));
    }
    let cipher = XChaCha20Poly1305::new(&symmetric_key(shared.as_bytes()).into());
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext.as_bytes(),
                aad,
            },
        )
        .expect("encrypt");
    let mut out = PublicKey::from(&ephemeral).as_bytes().to_vec();
    out.extend(nonce);
    out.extend(ciphertext);
    Ok(hex::encode(out))
}

fn open(hex: &str, secret: &StaticSecret, aad: &[u8]) -> Result<String> {
    let data = crate::nearai::bytes(hex)?;
    if data.len() < 72 {
        return Err(anyhow!("ciphertext too short"));
    }
    let ephemeral = PublicKey::from(<[u8; 32]>::try_from(&data[..32])?);
    let shared = secret.diffie_hellman(&ephemeral);
    if !shared.was_contributory() {
        return Err(anyhow!("weak encryption public key"));
    }
    let cipher = XChaCha20Poly1305::new(&symmetric_key(shared.as_bytes()).into());
    let plain = cipher
        .decrypt(
            XNonce::from_slice(&data[32..56]),
            Payload {
                msg: &data[56..],
                aad,
            },
        )
        .map_err(|_| anyhow!("could not decrypt a field"))?;
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
            ("x-encryption-version", "3".into()),
            ("x-encrypt-all-fields", "true".into()),
        ]
    }

    /// Only text messages are supported. Unknown content shapes must never
    /// fall through to the gateway as plaintext.
    pub fn encrypt_request(&self, messages: &[Value]) -> Result<Vec<Value>> {
        let aad = request_aad(self.client.verifying_key().as_bytes(), CONTENT_FIELD);
        messages
            .iter()
            .map(|message| {
                let content = message["content"]
                    .as_str()
                    .ok_or_else(|| anyhow!("only text messages are supported"))?;
                let role = message["role"]
                    .as_str()
                    .ok_or_else(|| anyhow!("message has no role"))?;
                Ok(json!({"role": role, "content": seal(content, &self.model_x25519, &aad)?}))
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
                content.push_str(&open(fragment, &self.client_x25519, &[])?);
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
        assert!(seal("private", &PublicKey::from([0; 32]), &[]).is_err());
        assert!(open(&hex::encode([0; 72]), &StaticSecret::from([7; 32]), &[]).is_err());
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
        let aad = request_aad(session.client.verifying_key().as_bytes(), CONTENT_FIELD);
        assert_eq!(
            open(messages[0]["content"].as_str().unwrap(), &secret, &aad).unwrap(),
            "private text"
        );
        let client = PublicKey::from(&session.client_x25519);
        let reply = session
            .decrypt_stream(&[
                json!({"id":"chat", "choices":[{"delta":{
                    "content":seal("first ", &client, &[]).unwrap(),
                    "reasoning_content":"unused reasoning is not decrypted"
                }}]}),
                json!({"choices":[{"delta":{"content":seal("second", &client, &[]).unwrap()}}]}),
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

    #[test]
    fn request_authentication_rejects_reply_key_field_and_version_changes() {
        let model = SigningKey::from_bytes(&[7; 32]);
        let session = Session::new(&hex::encode(model.verifying_key().as_bytes())).unwrap();
        let headers = session.headers();
        assert!(headers.contains(&("x-encryption-version", "3".into())));
        let reply_public: [u8; 32] = hex::decode(
            &headers
                .iter()
                .find(|(name, _)| *name == "x-client-pub-key")
                .unwrap()
                .1,
        )
        .unwrap()
        .try_into()
        .unwrap();
        let encrypted = session
            .encrypt_request(&[json!({"role":"user", "content":"private text"})])
            .unwrap();
        let ciphertext = encrypted[0]["content"].as_str().unwrap();
        let model_secret = StaticSecret::from(model.to_scalar_bytes());
        assert_eq!(
            open(
                ciphertext,
                &model_secret,
                &request_aad(&reply_public, CONTENT_FIELD)
            )
            .unwrap(),
            "private text"
        );
        let other_reply = SigningKey::from_bytes(&[8; 32]);
        for wrong in [
            request_aad(other_reply.verifying_key().as_bytes(), CONTENT_FIELD),
            request_aad(&reply_public, "tool_choice.function.name"),
            Vec::new(), // A gateway downgrading the version to v2 cannot open it.
        ] {
            assert!(open(ciphertext, &model_secret, &wrong).is_err());
        }
    }

    #[test]
    fn ed25519_request_matches_upstream_v3_vector() {
        // Public deterministic test material shared by the model proxy and
        // all four SDKs: nearai/inference-sdk@b72c3f9/test-fixtures/e2ee-v3.json.
        let seed = hex::decode("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
            .unwrap()
            .try_into()
            .unwrap();
        let model = SigningKey::from_bytes(&seed);
        assert_eq!(
            hex::encode(model.verifying_key().as_bytes()),
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
        );
        let reply_public =
            hex::decode("66be7e332c7a453332bd9d0a7f7db055f5c5ef1a06ada66d98b39fb6810c473a")
                .unwrap()
                .try_into()
                .unwrap();
        let aad = request_aad(&reply_public, CONTENT_FIELD);
        assert_eq!(hex::encode(&aad), "6e65617261692d653265652d726571756573742d76330266be7e332c7a453332bd9d0a7f7db055f5c5ef1a06ada66d98b39fb6810c473a006d657373616765735b5d2e636f6e74656e74");
        let ciphertext = "5dfedd3b6bd47f6fa28ee15d969d5bb0ea53774d488bdaf9df1c6e0124b3ef22000102030405060708090a0b0c0d0e0f1011121314151617952f1029d9facf0669005cd32fc9f3682f65e17b55d57abc02aeccf35eaabf4b8efe9979121175";
        assert_eq!(
            open(
                ciphertext,
                &StaticSecret::from(model.to_scalar_bytes()),
                &aad
            )
            .unwrap(),
            "E2EE v3: private prompt"
        );
        for wrong in [
            Vec::new(),
            request_aad(model.verifying_key().as_bytes(), CONTENT_FIELD),
            request_aad(&reply_public, "tool_choice.function.name"),
        ] {
            assert!(open(
                ciphertext,
                &StaticSecret::from(model.to_scalar_bytes()),
                &wrong
            )
            .is_err());
        }
    }
}
