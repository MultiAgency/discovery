//! Local organizer signing delegated to the official NEAR CLI. We neither
//! encode transactions nor sign them. A private recovery file survives an
//! uncertain broadcast so the caller can inspect the exact signed transaction.

use crate::{near, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const GAS: u64 = 300_000_000_000_000;

struct Scratch {
    dir: PathBuf,
    keep: bool,
}
impl Scratch {
    fn new() -> Result<Self> {
        let mut random = [0; 16];
        getrandom::fill(&mut random).map_err(|_| "signing randomness unavailable")?;
        let root = std::env::var_os("DISCOVERY_SIGNED_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let mut parent = fs::DirBuilder::new();
        parent.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            parent.mode(0o700);
        }
        parent.create(&root)?;
        let dir = root.join(format!("discovery-sign-{}", hex::encode(random)));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&dir)?;
        Ok(Self { dir, keep: false })
    }
    fn write(&self, name: &str, value: &Value) -> Result<PathBuf> {
        let path = self.dir.join(name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        file.write_all(&serde_json::to_vec(value)?)?;
        file.sync_all()?;
        Ok(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        // Even when retaining a signed recovery artifact, never retain a
        // temporary copy of the private organizer key.
        let _ = fs::remove_file(self.dir.join("key.json"));
        if !self.keep {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }
}

fn environment_key(organizer: &str, key: &str) -> Result<Value> {
    let encoded = key
        .trim()
        .strip_prefix("ed25519:")
        .ok_or("organizer key must be ed25519:<base58>")?;
    let bytes = bs58::decode(encoded)
        .into_vec()
        .map_err(|_| "organizer key is not valid base58")?;
    if bytes.len() != 64 {
        return Err("use a canonical 64-byte NEAR keypair, an access-key file, or the official CLI keychain".into());
    }
    // NEAR's canonical ed25519 keypair stores seed || public key. The official
    // CLI parses and signs with it; no signing implementation lives here.
    Ok(
        json!({"account_id":organizer,"public_key":format!("ed25519:{}",bs58::encode(&bytes[32..]).into_string()),"private_key":key.trim()}),
    )
}

fn command(
    program: &Path,
    network: &str,
    organizer: &str,
    call: &Value,
    args: &Path,
    output: &Path,
    key_file: Option<&Path>,
) -> Result<Command> {
    let receiver = call["receiver_id"].as_str().ok_or("call has no receiver")?;
    let method = call["method_name"].as_str().ok_or("call has no method")?;
    let deposit = call["deposit"].as_str().ok_or("call has no deposit")?;
    let gas: u64 = call["gas"]
        .as_str()
        .ok_or("call has no gas")?
        .parse()
        .map_err(|_| "invalid gas budget")?;
    if !gas.is_multiple_of(1_000_000_000_000) {
        return Err("gas budget must be a whole number of Tgas".into());
    }
    let mut cmd = Command::new(program);
    cmd.args([
        "--quiet",
        "contract",
        "call-function",
        "as-transaction",
        receiver,
        method,
        "file-args",
    ])
    .arg(args)
    .args([
        "prepaid-gas",
        &format!("{} Tgas", gas / 1_000_000_000_000),
        "attached-deposit",
        &format!("{deposit} yoctoNEAR"),
        "sign-as",
        organizer,
        "network-config",
        network,
    ]);
    match key_file {
        Some(path) => {
            cmd.arg("sign-with-access-key-file").arg(path);
        }
        None => {
            cmd.arg("sign-with-legacy-keychain");
        }
    }
    cmd.arg("save-to-file")
        .arg(output)
        .env_remove("DISCOVERY_ORGANIZER_KEY")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    Ok(cmd)
}

fn prepare(
    scratch: &Scratch,
    network: &str,
    organizer: &str,
    call: &Value,
    program: &Path,
    key_file: Option<&Path>,
) -> Result<PathBuf> {
    let args = scratch.write("call.json", &call["args"])?;
    let output = scratch.dir.join("signed.json");
    let mut child = command(program, network, organizer, call, &args, &output, key_file)?
        .spawn()
        .map_err(|_| "install the official near CLI (near-cli-rs 0.30.1) for organizer commands")?;
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                return Err("NEAR CLI could not sign; check its network configuration and organizer keychain/access-key file; nothing was broadcast".into());
            }
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("NEAR CLI signing timed out; nothing was broadcast".into());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    // The CLI creates this file inside a private directory. Restrict it too.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&output, fs::Permissions::from_mode(0o600))?;
    }
    read_signed(&output)?;
    Ok(output)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedFile {
    signed_transaction_as_base64: String,
}

pub fn read_signed(path: &Path) -> Result<String> {
    if fs::metadata(path)?.len() > 16 * 1024 * 1024 {
        return Err("signed transaction file is too large".into());
    }
    let file: SignedFile = serde_json::from_slice(&fs::read(path)?)
        .map_err(|_| "not an official CLI signed-transaction JSON file")?;
    if B64
        .decode(&file.signed_transaction_as_base64)
        .ok()
        .is_none_or(|b| b.len() < 65)
    {
        return Err(
            "signed transaction file contains invalid base64 or a truncated transaction".into(),
        );
    }
    Ok(file.signed_transaction_as_base64)
}

pub fn execute(
    rpc: &near::Rpc,
    network: &str,
    organizer: &str,
    call: &Value,
    resume: Option<&Path>,
) -> Result<Value> {
    if let Some(path) = resume {
        // Recovery is read-only: never re-sign or broadcast a replacement.
        let outcome = rpc.await_signed(&read_signed(path)?)?;
        check(&outcome, organizer, call)?;
        return Ok(outcome);
    }
    let mut scratch = Scratch::new()?;
    let configured = std::env::var_os("DISCOVERY_ORGANIZER_KEY_FILE").map(PathBuf::from);
    let environment = std::env::var("DISCOVERY_ORGANIZER_KEY")
        .ok()
        .filter(|k| !k.is_empty());
    if configured.is_some() && environment.is_some() {
        return Err("choose one organizer key source: file or environment".into());
    }
    let temporary = environment
        .as_ref()
        .map(|key| environment_key(organizer, key).and_then(|v| scratch.write("key.json", &v)))
        .transpose()?;
    let program = Path::new("near");
    let output = prepare(
        &scratch,
        network,
        organizer,
        call,
        program,
        configured.as_deref().or(temporary.as_deref()),
    )?;
    if temporary.is_some() {
        fs::remove_file(scratch.dir.join("key.json"))?;
    }
    let signed = read_signed(&output)?;
    scratch.keep = true;
    match rpc.submit_signed(&signed) {
        Ok(outcome) => {
            check(&outcome, organizer, call).map_err(|_| {
                format!(
                    "chain result differs from authorized call; signed transaction retained at {}",
                    output.display()
                )
            })?;
            scratch.keep = false;
            Ok(outcome)
        }
        Err(_) => Err(format!("submission outcome is uncertain; signed transaction retained at {}. Retry this operation with --resume-signed <that path> to query the same transaction; do not create a replacement until reconciled",output.display()).into()),
    }
}

pub fn call(contract: &str, args: &Value, deposit: u128) -> Value {
    json!({"receiver_id":contract,"method_name":"request_execution","args":args,"gas":GAS.to_string(),"deposit":deposit.to_string()})
}

pub fn check_payment_owner(organizer: &str, key: Option<&str>) -> Result<()> {
    if let Some(key) = key {
        let parts: Vec<_> = key.split(':').collect();
        if parts.len() != 3
            || parts[0] != organizer
            || parts[1].parse::<u64>().is_err()
            || parts[2].is_empty()
        {
            return Err("submission payment key must belong to the pinned organizer".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

fn check(outcome: &Value, organizer: &str, call: &Value) -> Result<()> {
    let hash = outcome["transaction"]["hash"]
        .as_str()
        .ok_or("transaction has no hash")?;
    if bs58::decode(hash)
        .into_vec()
        .ok()
        .is_none_or(|b| b.len() != 32)
    {
        return Err("transaction hash is invalid".into());
    }
    near::check_call(outcome, hash, organizer, call)
}
