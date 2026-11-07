//! The participant invitation, rendered from a survey this process just
//! verified. `scripts/invitation.sh` adds the checkout checks for `--commit`.
//!
//! Every survey value comes from the authenticated definition and the compiled
//! release; only the installation commit, the page URL and the published
//! payment key are supplied, and each is validated. Definition prose is
//! organizer-written and untrusted: it is rendered inert.

use discovery_protocol::definition::SurveyDefinition;

use crate::Result;

const TEMPLATE: &str = include_str!("../../../docs/invitation.md");
const NOTICE_END: &str = "-->\n\n";

/// Everything an invitation states. Built only by `main` after a successful
/// verification that showed the survey accepting responses.
#[derive(Clone, Copy)]
pub struct Invitation<'a> {
    pub definition: &'a SurveyDefinition,
    pub survey_id: &'a str,
    pub init_tx: &'a str,
    pub release: &'a str,
    pub payment_key: &'a str,
    pub commit: &'a str,
    pub page: &'a str,
}

/// Local checks for the supplied publication details, run before any network call.
pub fn check_details(commit: &str, page: &str) -> Result<()> {
    let mut errors = Vec::new();
    if commit.len() != 40
        || !commit
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        errors.push("--commit must be a full lowercase 40-character commit hash");
    }
    let rest = page.strip_prefix("https://").unwrap_or_default();
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    // Parsing a Request's URL is local: no request is sent. Do not let the URL
    // parser repair a missing authority (for example, https:///example.org),
    // or publish credentials disguised as the invitation's hostname.
    let parsed = ureq::get(page).request_url();
    let valid_url = parsed.is_ok_and(|url| {
        let url = url.as_url();
        url.scheme() == "https" && url.username().is_empty() && url.password().is_none()
    });
    if rest.is_empty()
        || authority.is_empty()
        || authority.contains('@')
        || !valid_url
        || !rest
            .bytes()
            .all(|b| b.is_ascii_graphic() && !b"<>()[]{}\"'`\\".contains(&b))
    {
        errors.push("--page must be the https:// URL where the invitation is published");
    }
    match errors.is_empty() {
        true => Ok(()),
        false => Err(errors.join("\n- ").into()),
    }
}

/// A published key is inserted into an unquoted shell assignment. Check its
/// printable alphabet before verification can make a paid service call.
pub fn check_payment_key(key: &str) -> Result<()> {
    if key.is_empty()
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b":._-".contains(&b))
    {
        return Err(
            "the payment key is empty or has characters unsafe to print in shell commands".into(),
        );
    }
    Ok(())
}

pub fn render(invitation: &Invitation) -> Result<String> {
    let d = invitation.definition;
    let repo_safe = d.target.repo.split_once('/').is_some_and(|(owner, name)| {
        [owner, name].iter().all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
    });
    if !repo_safe {
        return Err("the survey's target repository is not a plain GitHub owner/name".into());
    }
    check_payment_key(invitation.payment_key)?;
    check_details(invitation.commit, invitation.page)?;

    let target = format!("{}#{}", d.target.repo, d.target.number);
    let questions: Vec<String> = d
        .questions
        .iter()
        .enumerate()
        .map(|(i, q)| format!("{}. {}: {}", i + 1, question_id(&q.id), prose(&q.prompt)))
        .collect();
    let publishable = if d.rules.no_direct_quotes {
        "may appear paraphrased but never quoted, so write it as you'd be comfortable seeing it in public"
    } else {
        "may appear paraphrased or in short quotes, so write it as you'd be comfortable seeing it in public"
    };
    let withheld = match d.rules.withheld_terms.as_slice() {
        [] => String::new(),
        terms => format!(
            "- **Never in the report:** {}.\n",
            terms
                .iter()
                .map(|t| literal(t))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let value = |name: &str| -> Option<String> {
        Some(match name {
            "title" => prose(&d.title),
            "target" => target.clone(),
            "issue_url" => format!(
                "https://github.com/{}/issues/{}",
                d.target.repo, d.target.number
            ),
            "page_url" => invitation.page.into(),
            "commit" => invitation.commit.into(),
            "release" => invitation.release.into(),
            "survey_id" => invitation.survey_id.into(),
            "init_tx" => invitation.init_tx.into(),
            "payment_key" => invitation.payment_key.into(),
            "deadline" => utc(d.deadline_ns),
            "questions" => questions.join("\n"),
            "publishable" => publishable.into(),
            "withheld_terms" => withheld.clone(),
            _ => return None,
        })
    };

    let (_, body) = TEMPLATE
        .split_once(NOTICE_END)
        .ok_or("invitation template has no notice")?;
    let mut out = format!(
        "<!-- Generated by scripts/invitation.sh from a successful `discovery verify` of \
         survey {} at commit {}. Review before publishing. -->\n\n",
        invitation.survey_id, invitation.commit
    );
    // One pass, so substituted values are never scanned for placeholders.
    let mut rest = body;
    while let Some(start) = rest.find("{{") {
        out += &rest[..start];
        let (name, after) = rest[start + 2..]
            .split_once("}}")
            .ok_or("unterminated template placeholder")?;
        out += &value(name).ok_or_else(|| format!("unknown template placeholder {name}"))?;
        rest = after;
    }
    out += rest;
    Ok(out)
}

/// Organizer prose as one inert Markdown line: no HTML, no `@mentions` (which
/// notify people), no links, emphasis or code.
fn prose(text: &str) -> String {
    let mut out = String::new();
    for c in text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
    {
        match c {
            '&' => out += "&amp;",
            '<' => out += "&lt;",
            '>' => out += "&gt;",
            '@' => out += "&#64;",
            c if c.is_control()
                || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') =>
            {
                // Visible in Markdown and harmless when stdout is a terminal.
                out += &format!("\\\\u{:04x}", c as u32);
            }
            // Colon and dot also break GFM's bare URL and www. autolinks.
            '\\' | '`' | '*' | '_' | '[' | ']' | '#' | '~' | '|' | '!' | ':' | '.' => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

/// Plain ids stay easy to copy. Other ids are shown as explicitly labelled
/// JSON strings, so invisible characters cannot change what the reader copies.
fn question_id(text: &str) -> String {
    if text
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        code(text)
    } else {
        json_literal(text)
    }
}

/// Keep ordinary spaces in terms, while making any whitespace that code spans
/// would normalize or conceal (and every non-ASCII character) explicit.
fn literal(text: &str) -> String {
    if text.trim() != text
        || text.contains("  ")
        || text.chars().any(|c| c.is_control() || !c.is_ascii())
    {
        json_literal(text)
    } else {
        code(text)
    }
}

fn json_literal(text: &str) -> String {
    // JSON escapes control characters. Escape spaces and non-ASCII characters
    // too, so the displayed string remains exact even with bidi controls or
    // Markdown's code-span whitespace normalization.
    let json = serde_json::to_string(text).expect("serializing a string cannot fail");
    let mut visible = String::new();
    for c in json.chars() {
        if c == ' ' || !c.is_ascii() {
            for unit in c.encode_utf16(&mut [0; 2]) {
                visible += &format!("\\u{unit:04x}");
            }
        } else {
            visible.push(c);
        }
    }
    format!(
        "{} (JSON string; decode for the exact value)",
        code(&visible)
    )
}

/// A single-line literal as a code span. Callers make unusual whitespace and
/// invisible characters explicit first; never silently normalize identifiers.
fn code(text: &str) -> String {
    let longest = text
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    let fence = "`".repeat(longest + 1);
    let pad = if text.starts_with('`') || text.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{text}{pad}{fence}")
}

/// A NEAR block timestamp in UTC, keeping any sub-second nanoseconds.
fn utc(ns: u64) -> String {
    let (secs, nanos) = (ns / 1_000_000_000, ns % 1_000_000_000);
    let (days, day_secs) = ((secs / 86_400) as i64, secs % 86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let fraction = match nanos {
        0 => String::new(),
        n => format!(".{n:09}"),
    };
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}{fraction} UTC",
        day_secs / 3_600,
        day_secs / 60 % 60,
        day_secs % 60
    )
}

/// Rendering only. These values are FIXTURES, never a verification: the real
/// command builds an [`Invitation`] solely from `verify` in this process.
#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
    const FIXTURE_PAGE: &str = "https://example.invalid/fixture-invitation";

    fn fixture() -> SurveyDefinition {
        let mut d: SurveyDefinition =
            serde_json::from_str(include_str!("../../../fixtures/definition.json")).unwrap();
        d.deadline_ns = 1_800_000_000_123_456_789;
        d
    }

    fn render_fixture(d: &SurveyDefinition) -> Result<String> {
        render(&Invitation {
            definition: d,
            survey_id: &"f".repeat(64),
            init_tx: "FixtureCreationTx11111111111111111111111111",
            release: &"e".repeat(64),
            payment_key: "organizer.testnet:1:fixture",
            commit: FIXTURE_COMMIT,
            page: FIXTURE_PAGE,
        })
    }

    #[test]
    fn the_template_is_marked_and_fully_substituted() {
        assert!(TEMPLATE.starts_with("<!-- TEMPLATE. Not an invitation."));
        let page = render_fixture(&fixture()).unwrap();
        assert!(!page.contains("{{") && !page.contains("TEMPLATE"), "{page}");
        assert!(page.starts_with("<!-- Generated by scripts/invitation.sh"));
        for expected in [
            "git checkout 0123456789abcdef0123456789abcdef01234567",
            "blob/0123456789abcdef0123456789abcdef01234567/docs/policy.md",
            "blob/0123456789abcdef0123456789abcdef01234567/skills/discovery-respondent/SKILL.md",
            "--target MultiAgency/discovery#1",
            "https://github.com/MultiAgency/discovery/issues/1",
            "described at https://example.invalid/fixture-invitation.",
            "export DISCOVERY_PAYMENT_KEY=organizer.testnet:1:fixture",
            "deadline: 2027-01-15 08:00:00.123456789 UTC",
            "or in short quotes",
        ] {
            assert!(page.contains(expected), "{expected}");
        }
    }

    /// The invitation asks the authenticated questions, with Markdown punctuation
    /// escaped so the rendered text retains their wording.
    #[test]
    fn questions_come_from_the_definition_word_for_word() {
        let d = fixture();
        let page = render_fixture(&d).unwrap();
        for (i, q) in d.questions.iter().enumerate() {
            let line = format!("{}. `{}`: {}", i + 1, q.id, q.prompt.replace('.', "\\."));
            assert!(page.lines().any(|l| l == line), "{line}");
        }
    }

    #[test]
    fn restrictions_follow_the_definition() {
        let mut d = fixture();
        d.rules.no_direct_quotes = true;
        d.rules.withheld_terms = vec!["Project Falcon".into(), "a`b".into()];
        let page = render_fixture(&d).unwrap();
        assert!(page.contains("never quoted") && !page.contains("short quotes"));
        assert!(page.contains("- **Never in the report:** `Project Falcon`, ``a`b``.\n\n"));
    }

    #[test]
    fn untrusted_prose_is_rendered_inert() {
        let mut d = fixture();
        d.title = "Survey @octocat <img src=x> {{commit}}".into();
        d.questions[0].prompt =
            "Ping @team\n\n# Heading [link](https://evil.invalid) <!-- `x` --> *b* {{page_url}}"
                .into();
        d.questions[1].id = "`@id`".into();
        let page = render_fixture(&d).unwrap();
        for unsafe_text in [
            "@octocat",
            "@team",
            "<img",
            "<!-- `x`",
            "[link]",
            "\n# Heading",
            "*b*",
        ] {
            assert!(!page.contains(unsafe_text), "{unsafe_text}");
        }
        assert!(page.contains("Survey &#64;octocat &lt;img src=x&gt; {{commit}}"));
        assert!(page.contains("1. `workflow`: Ping &#64;team \\# Heading \\[link\\](https\\://evil\\.invalid) &lt;\\!-- \\`x\\` --&gt; \\*b\\* {{page\\_url}}"));
        assert!(page.contains("2. ``\"`@id`\"`` (JSON string; decode for the exact value): "));
        assert_eq!(
            prose("https://evil.invalid www.evil.invalid ftp://evil.invalid x@y.invalid"),
            "https\\://evil\\.invalid www\\.evil\\.invalid ftp\\://evil\\.invalid x&#64;y\\.invalid"
        );
    }

    #[test]
    fn unusual_question_ids_are_visible_json_without_losing_information() {
        let mut d = fixture();
        for (id, expected) in [
            (" a  b\tc\n", r#""\u0020a\u0020\u0020b\tc\n""#),
            ("x\u{202e}y", r#""x\u202ey""#),
            ("emoji🦀", r#""emoji\ud83e\udd80""#),
        ] {
            d.questions[1].id = id.into();
            let page = render_fixture(&d).unwrap();
            let line = format!("2. `{expected}` (JSON string; decode for the exact value): ");
            assert!(page.lines().any(|l| l.starts_with(&line)), "{page}");
            assert_eq!(serde_json::from_str::<String>(expected).unwrap(), id);
            assert!(!page.contains(id));
        }
        assert_eq!(question_id("plain-id_2.3"), "`plain-id_2.3`");
    }

    #[test]
    fn prose_controls_are_visible_without_altering_ordinary_unicode() {
        let mut d = fixture();
        d.title = "Survey \u{001b}[2J\u{202e}hidden".into();
        d.questions[0].prompt = "Prompt \u{0008}\u{2066}hidden".into();
        let page = render_fixture(&d).unwrap();
        for control in ['\u{001b}', '\u{202e}', '\u{0008}', '\u{2066}'] {
            assert!(!page.contains(control));
        }
        for visible in [r"\\u001b", r"\\u202e", r"\\u0008", r"\\u2066"] {
            assert!(page.contains(visible), "{visible}");
        }
        assert_eq!(prose("Arabic مرحبا"), "Arabic مرحبا");
    }

    #[test]
    fn unusual_withheld_terms_are_not_silently_rewritten() {
        let mut d = fixture();
        d.rules.withheld_terms = vec![" Project  Falcon\n".into(), "x\u{2066}y".into()];
        let page = render_fixture(&d).unwrap();
        assert!(page.contains(
            r#"`"\u0020Project\u0020\u0020Falcon\n"` (JSON string; decode for the exact value)"#
        ));
        assert!(page.contains(r#"`"x\u2066y"` (JSON string; decode for the exact value)"#));
    }

    #[test]
    fn unsafe_or_missing_publication_values_are_refused() {
        let d = fixture();
        let base = Invitation {
            definition: &d,
            survey_id: "s",
            init_tx: "t",
            release: "r",
            payment_key: "organizer.testnet:1:fixture",
            commit: FIXTURE_COMMIT,
            page: FIXTURE_PAGE,
        };
        assert!(render(&base).is_ok());
        for commit in ["", "HEAD", "0123456", &FIXTURE_COMMIT.to_uppercase()] {
            let error = render(&Invitation { commit, ..base }).unwrap_err();
            assert!(error.to_string().contains("--commit"), "{commit}");
        }
        for page in [
            "",
            "https://",
            "http://example.invalid/x",
            "https:///",
            "https:///example.invalid/x",
            "https://?x",
            "https://#fragment",
            "https://@example.invalid/x",
            "https://user@example.invalid/x",
            "https://user:secret@example.invalid/x",
            "https://example.invalid:port/x",
            "https://%/x",
            "https://example.invalid/a b",
            "https://example.invalid/<x>",
            "https://example.invalid/(x)",
        ] {
            let error = render(&Invitation { page, ..base }).unwrap_err();
            assert!(error.to_string().contains("--page"), "{page}");
        }
        let both = check_details("", "").unwrap_err().to_string();
        assert!(both.contains("--commit") && both.contains("--page"));
        let error = render(&Invitation {
            payment_key: "a:b:$(x)",
            ..base
        });
        assert!(error.is_err());
        let mut bad = d.clone();
        bad.target.repo = "owner/na)me".into();
        assert!(render(&Invitation {
            definition: &bad,
            ..base
        })
        .is_err());
    }

    #[test]
    fn publication_key_checks_are_local_and_never_echo_inputs() {
        assert!(check_payment_key("organizer.testnet:1:fixture_-KEY").is_ok());
        for key in [
            "",
            "a:b:$(SECRET)",
            "a:b:SECRET\n",
            "a:b:'SECRET'",
            "a:b:SECRÉT",
        ] {
            let error = check_payment_key(key).unwrap_err().to_string();
            assert!(error.contains("payment key"));
            assert!(!error.contains("SECRET") && !error.contains("SECRÉT"));
        }
    }

    #[test]
    fn deadlines_keep_nanosecond_precision() {
        assert_eq!(utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(utc(951_782_400 * 1_000_000_000), "2000-02-29 00:00:00 UTC");
        assert_eq!(
            utc(1_800_000_000_000_000_001),
            "2027-01-15 08:00:00.000000001 UTC"
        );
        assert_eq!(utc(u64::MAX), "2554-07-21 23:34:33.709551615 UTC");
    }
}
