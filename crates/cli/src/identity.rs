//! Private local identity, with a separate signing key for each survey.

use crate::Result;
use discovery_protocol::identity::{RespondentKey, Submission};
use discovery_protocol::response::Response;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::Path;

const SECRET: &str = "respondent-secret";
fn read_secret(home: &Path) -> Result<[u8; 32]> {
    let text = fs::read_to_string(home.join(SECRET))?;
    Ok(hex::decode(text.trim())?
        .try_into()
        .map_err(|_| "identity secret is not 32 bytes")?)
}

/// Create with private permissions and exclusive creation; never replace an
/// unreadable, malformed or concurrently created identity.
fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn local_secret(home: &Path) -> Result<[u8; 32]> {
    match fs::read_to_string(home.join(SECRET)) {
        Ok(_) => return read_secret(home),
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    fs::create_dir_all(home)?;
    let mut secret = [0; 32];
    getrandom::fill(&mut secret).map_err(|_| "identity randomness unavailable")?;
    match write_new(&home.join(SECRET), hex::encode(secret).as_bytes()) {
        Ok(()) => Ok(secret),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => read_secret(home),
        Err(e) => Err(e.into()),
    }
}

pub fn sign_submission(home: &Path, response: Response) -> Result<Submission> {
    let hash = hex::decode(&response.definition_hash)?
        .try_into()
        .map_err(|_| "invalid definition hash")?;
    Ok(RespondentKey::for_survey(&local_secret(home)?, &hash)?.sign(response)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use discovery_protocol::response::{Answer, Basis, Disclosure};
    use std::path::PathBuf;

    struct Home(PathBuf);
    impl Home {
        fn new() -> Self {
            let mut random = [0; 16];
            getrandom::fill(&mut random).unwrap();
            let path = std::env::temp_dir()
                .join(format!("discovery-identity-test-{}", hex::encode(random)));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Home {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn response(seed: u8) -> Response {
        Response {
            definition_hash: hex::encode([seed; 32]),
            sent_at_ms: 1,
            answers: vec![Answer {
                question_id: "q".into(),
                basis: Basis::Observed,
                disclosure: Disclosure::Publishable,
                text: "An observed failure".into(),
                evidence: vec![],
                repro_steps: vec![],
            }],
        }
    }
    #[test]
    fn edits_keep_identity_but_other_surveys_get_different_keys() {
        let home = Home::new();
        let first = sign_submission(&home.0, response(1)).unwrap();
        let edit = sign_submission(&home.0, response(1)).unwrap();
        let other = sign_submission(&home.0, response(2)).unwrap();
        assert_eq!(first.signer, edit.signer);
        assert_ne!(first.signer, other.signer);
        first.verify().unwrap();
    }

    #[test]
    fn identity_creation_is_private_and_never_overwrites_corrupt_keys() {
        let home = Home::new();
        let secret = local_secret(&home.0).unwrap();
        assert_eq!(local_secret(&home.0).unwrap(), secret);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(home.0.join(SECRET))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        fs::write(home.0.join(SECRET), "broken identity").unwrap();
        assert!(local_secret(&home.0).is_err());
        assert_eq!(
            fs::read_to_string(home.0.join(SECRET)).unwrap(),
            "broken identity"
        );
    }
}
