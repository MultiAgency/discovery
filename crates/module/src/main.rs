//! OutLayer WASI module for confidential surveys.
//!
//! Input: `{"operation": "init_survey" | "submit" | "status" | "finalize", ...}`.
//! Output: the operation's result, or `{"error": "..."}`.

mod checks;
mod collection;
mod finalize;
mod nearai;
mod platform;
mod survey;

use discovery_protocol::definition::SurveyDefinition;
use discovery_protocol::Result;
use outlayer::env;
use serde::Deserialize;
use serde_json::{json, Value};

// The manifest lives in the `outlayer.manifest` custom section, covered by the
// wasm's sha256.
#[cfg(target_arch = "wasm32")]
#[used]
#[link_section = "outlayer.manifest"]
static OUTLAYER_MANIFEST: [u8; include_bytes!("../manifest.json").len()] =
    *include_bytes!("../manifest.json");

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Input {
    InitSurvey { definition: SurveyDefinition },
    Submit { survey_id: String, envelope: String },
    Status { survey_id: String },
    Finalize { survey_id: String },
}

fn run() -> Result<Value> {
    let input: Input =
        env::input_json()?.ok_or_else(|| discovery_protocol::Error("no input".into()))?;
    let p = platform::Host;
    Ok(match input {
        Input::InitSurvey { definition } => serde_json::to_value(survey::init(&p, definition)?)?,
        Input::Submit {
            survey_id,
            envelope,
        } => serde_json::to_value(survey::submit(&p, &survey_id, &envelope)?)?,
        Input::Status { survey_id } => serde_json::to_value(survey::status(&p, &survey_id)?)?,
        Input::Finalize { survey_id } => serde_json::to_value(finalize::finalize(
            &p,
            &nearai::Confidential::new(),
            &survey_id,
        )?)?,
    })
}

fn main() {
    let answer = run().unwrap_or_else(|e| json!({ "error": e.0 }));
    env::output_json(&answer).expect("serialize output");
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    #[test]
    fn manifest_requires_direct_organizer_authority_and_allows_https_responses() {
        let manifest: Value = serde_json::from_str(include_str!("../manifest.json")).unwrap();
        // The worker enforces this before decrypting secrets or running code.
        // A relaying contract retains its transaction's original signer, so
        // accepting that door would let it close the signer's real collection.
        assert_eq!(
            manifest["callers"],
            json!({
                "direct": "allow",
                "contract": "deny",
                "https": "allow",
                "meta_tx": "deny"
            })
        );
    }
}

// Last, so release builds keep the same line numbers in panic locations.
#[cfg(test)]
mod demo;
