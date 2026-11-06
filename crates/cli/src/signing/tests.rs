use super::*;
use std::io::Read;
use std::net::TcpListener;

fn call_fixture() -> Value {
    call("outlayer.testnet", &json!({"input_data":"{}"}), 1)
}
fn hash() -> String {
    bs58::encode([9; 32]).into_string()
}
fn outcome() -> Value {
    let call = call_fixture();
    json!({"transaction":{"hash":hash(),"signer_id":"service.testnet","receiver_id":"outlayer.testnet",
        "actions":[{"FunctionCall":{"method_name":"request_execution","args":B64.encode(call["args"].to_string()),"gas":GAS,"deposit":"1"}}]},
        "status":{"SuccessValue":B64.encode("{}")}})
}

#[test]
fn official_cli_command_signs_to_file_with_exact_on_chain_parameters() {
    let call = call_fixture();
    let cmd = command(
        Path::new("near"),
        "testnet",
        "service.testnet",
        &call,
        Path::new("call.json"),
        Path::new("signed.json"),
        Some(Path::new("key.json")),
    )
    .unwrap();
    let args: Vec<_> = cmd.get_args().map(|v| v.to_str().unwrap()).collect();
    assert!(args.windows(2).any(|a| a == ["file-args", "call.json"]));
    assert!(args.windows(2).any(|a| a == ["prepaid-gas", "300 Tgas"]));
    assert!(args
        .windows(2)
        .any(|a| a == ["attached-deposit", "1 yoctoNEAR"]));
    assert!(args
        .windows(2)
        .any(|a| a == ["sign-with-access-key-file", "key.json"]));
    assert!(args.ends_with(&["save-to-file", "signed.json"]));
    assert!(!args.contains(&"send"));
    assert!(cmd
        .get_envs()
        .any(|(key, value)| key == "DISCOVERY_ORGANIZER_KEY" && value.is_none()));
}

#[test]
fn ci_credential_conversion_accepts_canonical_keys_without_signing() {
    let key = format!("ed25519:{}", bs58::encode([1; 64]).into_string());
    let file = environment_key("service.testnet", &key).unwrap();
    assert_eq!(file["account_id"], "service.testnet");
    assert_eq!(
        file["public_key"],
        format!("ed25519:{}", bs58::encode([1; 32]).into_string())
    );
    assert!(environment_key("service.testnet", "not a key").is_err());
    assert!(environment_key(
        "service.testnet",
        &format!("ed25519:{}", bs58::encode([1; 32]).into_string())
    )
    .is_err());
}

#[test]
fn private_key_copy_is_erased_even_when_signed_recovery_file_is_retained() {
    let path;
    {
        let mut scratch = Scratch::new().unwrap();
        path = scratch.dir.clone();
        scratch
            .write("key.json", &json!({"private_key":"synthetic"}))
            .unwrap();
        let signed = scratch
            .write(
                "signed.json",
                &json!({"signed_transaction_as_base64":B64.encode([1;100])}),
            )
            .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&signed).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(read_signed(&signed).is_ok());
        scratch.keep = true;
    }
    assert!(!path.join("key.json").exists());
    assert!(path.join("signed.json").exists());
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn malformed_official_cli_outputs_are_rejected_before_submission() {
    let scratch = Scratch::new().unwrap();
    for (i, value) in [
        json!({"private_key":"must not be interpreted as signed data"}),
        json!({"signed_transaction_as_base64":"bad"}),
        json!({"signed_transaction_as_base64":B64.encode([1;3])}),
    ]
    .iter()
    .enumerate()
    {
        let path = scratch.write(&format!("bad{i}.json"), value).unwrap();
        assert!(read_signed(&path).is_err());
    }
}

#[test]
fn exact_transaction_binding_rejects_substituted_calls() {
    let call = call_fixture();
    let valid = outcome();
    near::check_call(&valid, &hash(), "service.testnet", &call).unwrap();
    for field in ["hash", "signer_id", "receiver_id"] {
        let mut bad = valid.clone();
        bad["transaction"][field] = json!("other");
        assert!(near::check_call(&bad, &hash(), "service.testnet", &call).is_err());
    }
    for field in ["args", "method_name", "gas", "deposit"] {
        let mut bad = valid.clone();
        bad["transaction"]["actions"][0]["FunctionCall"][field] = json!("other");
        assert!(near::check_call(&bad, &hash(), "service.testnet", &call).is_err());
    }
    let mut bad = valid;
    bad["transaction"]["actions"]
        .as_array_mut()
        .unwrap()
        .push(json!({"Transfer":{"deposit":"1"}}));
    assert!(near::check_call(&bad, &hash(), "service.testnet", &call).is_err());
    assert!(check_payment_owner("service.testnet", Some("other.testnet:1:synthetic")).is_err());
}

fn mock(replies: Vec<Value>) -> (near::Rpc, std::thread::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for reply in replies {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let (end, length) = loop {
                let mut chunk = [0; 4096];
                let n = stream.read(&mut chunk).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    break (end + 4, length);
                }
            };
            while bytes.len() < end + length {
                let mut chunk = [0; 4096];
                let n = stream.read(&mut chunk).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
            }
            seen.push(serde_json::from_slice(&bytes[end..end + length]).unwrap());
            let body = reply.to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        }
        seen
    });
    (near::Rpc { url }, task)
}

#[test]
fn ambiguous_broadcast_queries_the_same_signed_bytes_without_resubmitting() {
    let signed = B64.encode([1; 100]);
    let (rpc, server) = mock(vec![
        json!({"error":{"message":"timeout"}}),
        json!({"result":outcome()}),
    ]);
    rpc.submit_signed(&signed).unwrap();
    let seen = server.join().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0]["method"], "send_tx");
    assert_eq!(seen[1]["method"], "tx");
    assert_eq!(seen[0]["params"]["signed_tx_base64"], signed);
    assert_eq!(seen[1]["params"]["signed_tx_base64"], signed);
    assert_eq!(seen[1]["params"]["wait_until"], "FINAL");
}

#[test]
fn recovery_is_read_only_and_still_checks_the_authorized_call() {
    let scratch = Scratch::new().unwrap();
    let signed = scratch
        .write(
            "signed.json",
            &json!({"signed_transaction_as_base64":B64.encode([1;100])}),
        )
        .unwrap();
    let (rpc, server) = mock(vec![json!({"result":outcome()})]);
    execute(
        &rpc,
        "testnet",
        "service.testnet",
        &call_fixture(),
        Some(&signed),
    )
    .unwrap();
    let seen = server.join().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0]["method"], "tx");
}

#[cfg(unix)]
#[test]
fn cli_output_is_read_from_its_file_not_parsed_from_terminal_text() {
    use std::os::unix::fs::PermissionsExt;
    let scratch = Scratch::new().unwrap();
    let program = scratch.dir.join("fake-near");
    fs::write(&program,format!("#!/usr/bin/env python3\nimport json,sys\njson.dump({{'signed_transaction_as_base64':'{}'}},open(sys.argv[-1],'w'))\nprint('unstructured CLI output')\n",B64.encode([1;100]))).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    let output = prepare(
        &scratch,
        "testnet",
        "service.testnet",
        &call_fixture(),
        &program,
        None,
    )
    .unwrap();
    assert!(read_signed(&output).is_ok());
}

#[test]
#[ignore = "uses the installed official CLI and a configured testnet keychain; no broadcast"]
fn official_cli_signs_without_broadcasting() {
    let rpc = near::Rpc {
        url: "https://archival-rpc.testnet.near.org".into(),
    };
    for use_file in [false, true] {
        let scratch = Scratch::new().unwrap();
        let key_file = if use_file {
            let original: Value = serde_json::from_slice(
                &fs::read(
                    PathBuf::from(std::env::var_os("HOME").unwrap())
                        .join(".near-credentials/testnet/research.agency.testnet.json"),
                )
                .unwrap(),
            )
            .unwrap();
            let value = environment_key(
                "research.agency.testnet",
                original["private_key"].as_str().unwrap(),
            )
            .unwrap();
            Some(scratch.write("key.json", &value).unwrap())
        } else {
            None
        };
        let call = call(
            "outlayer.testnet",
            &json!({"input_data":"{}"}),
            100_000_000_000_000_000_000_000,
        );
        let output = prepare(
            &scratch,
            "testnet",
            "research.agency.testnet",
            &call,
            Path::new("near"),
            key_file.as_deref(),
        )
        .unwrap();
        let signed = read_signed(&output).unwrap();
        let error = rpc.signed_status(&signed, "NONE").unwrap_err().to_string();
        assert!(
            error.contains("UNKNOWN_TRANSACTION"),
            "RPC did not recognize the signed-transaction status request"
        );
    }
    println!("Official CLI keychain and private-file signing verified; RPC accepts signed-byte status queries; no broadcast was requested.");
}
