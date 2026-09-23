use std::path::PathBuf;

use gaze::LocaleTag;
use gaze_assembly::CorePipelineConfig;

#[derive(Debug)]
pub struct EncodedOutput {
    pub redacted_text: String,
    pub mappings: Vec<TokenMapping>,
}

#[derive(Debug, Clone)]
pub struct TokenMapping {
    pub token: String,
    pub value: String,
    pub class: String,
    /// Byte offset of the value's first occurrence in the source text (display sort key).
    pub first_offset: usize,
}

#[derive(Debug, PartialEq)]
pub struct DecodedOutput {
    pub restored_text: String,
    pub hallucinations: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub enum VaultError {
    Pipeline(String),
    Session(String),
    Decode(String),
}

pub struct Vault {
    session: gaze::Session,
    pipeline: gaze::Pipeline,
}

/// A gaze token span in redacted text, including any source angle-bracket wrap.
struct GazeToken {
    /// Byte range covering the full span to replace (may include outer `<>`).
    span: std::ops::Range<usize>,
    /// Exact substring at `span` (e.g. `<hex:Email_1>` or `<<hex:Email_1>>`).
    raw: String,
    /// Canonical session key ` <hex:Class_N> ` used for `session.restore`.
    session_key: String,
    /// Readable class name (`Email_1`, `Custom:phone_1`, …).
    class: String,
}

fn is_session_token_inner(inner: &str) -> bool {
    let Some(colon) = inner.find(':') else {
        return false;
    };
    let (hex, class) = (&inner[..colon], &inner[colon + 1..]);
    !hex.is_empty()
        && hex.chars().all(|c| c.is_ascii_hexdigit())
        && class.contains('_')
        // ANYTHING after the session hex: letters, digits, colons, hyphens, underscores.
        && class
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':' || c == '-')
        && class.chars().last().is_some_and(|c| c.is_ascii_digit())
}

/// Last `_N` segment after the final `:` or `-`, first letter capitalised.
/// `Email_4` → `Email_4`; `Custom:date_1` → `Date_1`;
/// `Custom:family:payment-card-or-iban_1` → `Iban_1`.
fn readable_class(raw_class: &str) -> String {
    let last = raw_class
        .rsplit([':', '-'])
        .find(|segment| !segment.is_empty())
        .unwrap_or(raw_class);
    let mut chars = last.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => raw_class.to_string(),
    }
}

/// Turn a full gaze token (or raw class) into the readable `[Iban_1]` form.
pub fn readable_token(token_or_class: &str) -> String {
    let mut inner = token_or_class.trim();
    while inner.starts_with('<') {
        inner = &inner[1..];
    }
    while inner.ends_with('>') {
        inner = &inner[..inner.len() - 1];
    }
    let class = match inner.find(':') {
        Some(i) => &inner[i + 1..],
        None => inner,
    };
    format!("[{}]", readable_class(class))
}

/// Strip a leading name title so "Dr. Sarah Mitchell" and "Sarah Mitchell" link.
fn person_core_name(value: &str) -> &str {
    const TITLES: [&str; 12] = [
        "Dr. ", "Dr ", "Mr. ", "Mr ", "Ms. ", "Ms ", "Mrs. ", "Mrs ", "Mx. ", "Mx ", "Prof. ",
        "Prof ",
    ];
    for title in TITLES {
        if let Some(rest) = value.strip_prefix(title) {
            return rest;
        }
    }
    value
}

/// Find the next gaze token at or after `from`.
/// Matches `<hex:Class_N>` and also `<<hex:Class_N>>` (source email wrapped in `<>`).
fn next_gaze_token(text: &str, from: usize) -> Option<GazeToken> {
    let bytes = text.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }

        // `<<hex:Class_N>>` → core token starts at the second `<`.
        let double = i + 1 < bytes.len() && bytes[i + 1] == b'<';
        let core_start = if double { i + 1 } else { i };

        if bytes.get(core_start) != Some(&b'<') {
            i += 1;
            continue;
        }
        let Some(rel_close) = text[core_start + 1..].find('>') else {
            break;
        };
        let core_end = core_start + 1 + rel_close; // index of `>`
        let inner = &text[core_start + 1..core_end];
        if !is_session_token_inner(inner) {
            i += 1;
            continue;
        }

        // Consume the outer source `<>` pair when present (`<<…>>`).
        let start = if double { i } else { core_start };
        let mut end = core_end + 1;
        if double && bytes.get(end) == Some(&b'>') {
            end += 1;
        }

        let colon = inner.find(':')?;
        return Some(GazeToken {
            raw: text[start..end].to_string(),
            session_key: format!("<{inner}>"),
            class: inner[colon + 1..].to_string(),
            span: start..end,
        });
    }
    None
}

impl Vault {
    pub fn new() -> Result<Self, VaultError> {
        // En-US activates locale-gated detectors (US phone, SSN). locale-en supplies
        // name cue packs. poc_extra adds free-text person / date / location rules.
        let extra_rules = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("rules")
            .join("poc_extra.toml");

        let pipeline = CorePipelineConfig::new()
            .with_locale(&[LocaleTag::EnUs])
            .with_bundled_rulepack("locale-en")
            .with_rulepack_path(extra_rules)
            .build()
            .map_err(|e| VaultError::Pipeline(e.to_string()))?
            .pipeline()
            .clone();

        let session = gaze::Session::new(gaze::Scope::Conversation("poc-session".into()))
            .map_err(|e| VaultError::Session(e.to_string()))?;

        Ok(Vault { session, pipeline })
    }

    pub fn encode(&self, text: &str) -> Result<EncodedOutput, VaultError> {
        let result = self.pipeline.redact(&self.session, gaze::RawDocument::Text(text.to_string()))
            .map_err(|e| VaultError::Pipeline(e.to_string()))?;

        let gaze::CleanDocument::Text(clean_text) = result else {
            return Err(VaultError::Pipeline("Expected Text document".into()));
        };

        let mut mappings: Vec<TokenMapping> = Vec::new();
        let mut readable_redacted_text = clean_text.clone();

        // Gaze emits <session_hex:Class_N>. When the source span was already wrapped
        // in <angle brackets>, the redacted form is <<session_hex:Class_N>>.
        let mut cursor = 0;
        while let Some(tok) = next_gaze_token(&clean_text, cursor) {
            let class_name = readable_class(&tok.class);
            let readable_token = format!("[{}]", class_name);

            let original_value = self
                .session
                .restore(&tok.session_key)
                .or_else(|| self.session.restore(&tok.raw))
                .map(|v| v.to_string());

            if let Some(original_value) = original_value {
                let first_offset = text.find(&original_value).unwrap_or(usize::MAX);
                if let Some(existing) = mappings
                    .iter_mut()
                    .find(|m| m.token == tok.session_key)
                {
                    if first_offset < existing.first_offset {
                        existing.first_offset = first_offset;
                    }
                } else {
                    mappings.push(TokenMapping {
                        token: tok.session_key.clone(),
                        value: original_value,
                        class: class_name,
                        first_offset,
                    });
                }
                readable_redacted_text = readable_redacted_text.replace(&tok.raw, &readable_token);
            }

            cursor = tok.span.end;
        }

        // Strip any surrounding < > left after replacing wrapped tokens (e.g. `<[Email_1]>`).
        readable_redacted_text = readable_redacted_text
            .replace("<<[", "[")
            .replace("]>>", "]")
            .replace("<[", "[")
            .replace("]>", "]");

        // Session accumulation: one readable token per distinct original value, so
        // repeated mentions share [Class_N] even if gaze emitted sibling counters.
        let mut value_to_class: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for m in &mappings {
            let entry = value_to_class.entry(m.value.clone()).or_insert_with(|| m.class.clone());
            if *entry != m.class {
                let from = format!("[{}]", m.class);
                let to = format!("[{entry}]");
                readable_redacted_text = readable_redacted_text.replace(&from, &to);
            }
        }
        let mut deduped: Vec<TokenMapping> = Vec::new();
        for m in mappings {
            let canonical = value_to_class
                .get(&m.value)
                .cloned()
                .unwrap_or_else(|| m.class.clone());
            if let Some(existing) = deduped.iter_mut().find(|d| d.value == m.value) {
                if m.first_offset < existing.first_offset {
                    existing.first_offset = m.first_offset;
                }
                existing.class = canonical;
            } else {
                deduped.push(TokenMapping {
                    token: m.token,
                    value: m.value,
                    class: canonical,
                    first_offset: m.first_offset,
                });
            }
        }

        // Session / entity accumulation for person names: once "Dr. Sarah Mitchell"
        // is mapped, remaining bare mentions of "Sarah Mitchell" in the same document
        // must reuse that token instead of leaking or getting a fresh counter.
        for m in &deduped {
            let is_name = m.class.contains("Name") || m.class.contains("name");
            if !is_name {
                continue;
            }
            let core = person_core_name(&m.value);
            if core.is_empty() || core == m.value {
                continue;
            }
            let token = format!("[{}]", m.class);
            if readable_redacted_text.contains(core) {
                readable_redacted_text = readable_redacted_text.replace(core, &token);
            }
        }

        // Strip any surrounding < > left after replacing wrapped tokens (e.g. `<[Email_1]>`).
        readable_redacted_text = readable_redacted_text
            .replace("<<[", "[")
            .replace("]>>", "]")
            .replace("<[", "[")
            .replace("]>", "]");

        // CHANGE 2: display order follows first appearance in the source text.
        deduped.sort_by_key(|m| m.first_offset);

        Ok(EncodedOutput {
            redacted_text: readable_redacted_text,
            mappings: deduped,
        })
    }

    pub fn decode(&self, llm_response: &str, mappings: &[TokenMapping]) -> Result<DecodedOutput, VaultError> {
        let mut replacements: Vec<(String, String)> = Vec::new();
        let mut hallucinations: Vec<String> = Vec::new();

        let mut start = 0;
        let llm_len = llm_response.len();

        while start < llm_len {
            if let Some(open) = llm_response[start..].find('[') {
                let open_idx = start + open;
                if let Some(close) = llm_response[open_idx..].find(']') {
                    let close_idx = open_idx + close;
                    let readable_token = &llm_response[open_idx..=close_idx];
                    let class_name = &llm_response[open_idx + 1..close_idx];

                    // Accept both [EMAIL_1] and gaze forms such as [Email_1] / [Custom:credit_card_1].
                    let is_valid_token = !class_name.is_empty()
                        && class_name
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
                        && class_name.contains('_')
                        && class_name.chars().last().is_some_and(|c| c.is_ascii_digit());

                    if is_valid_token {
                        if let Some(mapping) = mappings.iter().find(|m| m.class == class_name) {
                            // Prefer the captured original value; session.restore is only a fallback.
                            let original_value = self
                                .session
                                .restore(&mapping.token)
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| mapping.value.clone());

                            if !replacements.iter().any(|(t, _)| t == readable_token) {
                                replacements.push((readable_token.to_string(), original_value));
                            }
                        } else if !hallucinations.contains(&readable_token.to_string()) {
                            hallucinations.push(readable_token.to_string());
                        }
                    }

                    start = close_idx + 1;
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        // Replace longer tokens first so [Email_10] is not clobbered by [Email_1].
        replacements.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        let mut restored_text = llm_response.to_string();
        for (token, value) in replacements {
            restored_text = restored_text.replace(&token, &value);
        }

        Ok(DecodedOutput {
            restored_text,
            hallucinations,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors pii-tool/sample.txt (the document that missed bare "Priya Patel").
    const SAMPLE: &str = "\
Subject: Confidential: Employee Background Check and Onboarding Details
Date: 2026-09-24
To Human Resources:
We are processing the onboarding for our new hire, Dr. Sarah Mitchell.
Her contact information is as follows:
Phone: (555) 867-5309
Email: sarah.mitchell@example.com
Please verify her identity using her provided SSN: 123-45-6789.
Additionally, for the corporate expenses, we have authorized the use of the corporate card: 4111-1111-1111-1111 under the name Michael Chen.
All hardware shipments should be sent to her permanent address:
42 Beacon Street, Boston, MA 02108
If you need further details, please reach out to the hiring manager, Priya Patel at priya.patel@university.edu.
Note that the internal tracking code for this onboarding process is ABC-1234 and the portal version is 1.2.3.
Thank you,
HR Operations Team
";

    /// Compact multi-class fixture (emails + card + non-PII noise).
    const FIXTURE: &str = "Please contact Dr. Sarah Mitchell at sarah.mitchell@example.com \
or call her at (555) 867-5309 regarding the account for John A. Doe, SSN 123-45-6789. \
The meeting is scheduled for 2024-03-15 at our Boston office located at 42 Beacon Street, Boston, MA 02108. \
Our finance team can be reached at finance@acme-corp.org. \
Credit card on file ends with 4111-1111-1111-1111 under the name Michael Chen. \
Please CC Ms. Priya Patel (priya.patel@university.edu). \
Note that version 1.2.3 of the API and the product code ABC-1234 should remain unchanged.";

    fn map_token(class: &str, value: &str) -> TokenMapping {
        TokenMapping {
            token: format!("<test:{}>", class),
            value: value.to_string(),
            class: class.to_string(),
            first_offset: 0,
        }
    }

    fn assert_detected(encoded: &EncodedOutput, needle: &str, label: &str) {
        assert!(
            encoded.mappings.iter().any(|m| m.value.contains(needle)),
            "{label} not detected (wanted value containing {needle:?}): {:?}",
            encoded.mappings
        );
        assert!(
            !encoded.redacted_text.contains(needle),
            "{label} leaked into redacted text: {}",
            encoded.redacted_text
        );
    }

    fn assert_preserved(encoded: &EncodedOutput, needle: &str, label: &str) {
        assert!(
            encoded.redacted_text.contains(needle),
            "{label} was corrupted/removed during encode: {}",
            encoded.redacted_text
        );
    }

    fn assert_round_trips(input: &str) -> EncodedOutput {
        let vault = Vault::new().unwrap();
        let encoded = vault.encode(input).unwrap();
        let decoded = vault
            .decode(&encoded.redacted_text, &encoded.mappings)
            .unwrap();
        assert_eq!(
            decoded.restored_text, input,
            "round-trip must restore the original text exactly"
        );
        assert!(decoded.hallucinations.is_empty());
        encoded
    }

    // ---------- encode / decode core ----------

    #[test]
    fn t01_encode_redacts_email_and_records_mapping() {
        let vault = Vault::new().unwrap();
        let encoded = vault
            .encode("Contact sarah.mitchell@example.com today")
            .unwrap();

        assert_detected(&encoded, "sarah.mitchell@example.com", "email");
        assert!(
            encoded.redacted_text.contains('[') && encoded.redacted_text.contains(']'),
            "expected a readable token in: {}",
            encoded.redacted_text
        );
        assert_eq!(encoded.mappings.len(), 1);
    }

    #[test]
    fn t02_encode_decode_round_trip_restores_original() {
        assert_round_trips("Email alice.smith@corp.com about the invoice.");
    }

    #[test]
    fn t03_encode_multiple_entities_get_unique_tokens() {
        let vault = Vault::new().unwrap();
        let encoded = vault
            .encode("Mail a@x.com and b@y.com and c@z.com.")
            .unwrap();

        assert!(encoded.mappings.len() >= 2, "{:?}", encoded.mappings);
        let mut tokens: Vec<_> = encoded.mappings.iter().map(|m| m.token.clone()).collect();
        tokens.sort();
        tokens.dedup();
        assert_eq!(tokens.len(), encoded.mappings.len());
        let mut classes: Vec<_> = encoded.mappings.iter().map(|m| m.class.clone()).collect();
        classes.sort();
        classes.dedup();
        assert_eq!(classes.len(), encoded.mappings.len());
    }

    #[test]
    fn t04_decode_restores_token_appearing_multiple_times() {
        let vault = Vault::new().unwrap();
        let mappings = vec![map_token("Email_1", "alice@example.com")];
        let llm = "Send [Email_1] a copy and CC [Email_1] again.";

        let decoded = vault.decode(llm, &mappings).unwrap();
        assert_eq!(
            decoded.restored_text,
            "Send alice@example.com a copy and CC alice@example.com again."
        );
        assert!(decoded.hallucinations.is_empty());
    }

    #[test]
    fn t05_decode_detects_hallucinated_token() {
        let vault = Vault::new().unwrap();
        let mappings = vec![map_token("Email_1", "alice@example.com")];
        let llm = "Contact [PERSON_99] and [Email_42] immediately.";

        let decoded = vault.decode(llm, &mappings).unwrap();
        assert!(decoded.hallucinations.contains(&"[PERSON_99]".to_string()));
        assert!(decoded.hallucinations.contains(&"[Email_42]".to_string()));
        assert!(decoded.restored_text.contains("[PERSON_99]"));
        assert!(decoded.restored_text.contains("[Email_42]"));
    }

    #[test]
    fn t06_decode_leaves_non_token_brackets_untouched() {
        let vault = Vault::new().unwrap();
        let llm = "See [note] and [optional] and [see page 2] and [N/A] and [].";

        let decoded = vault.decode(llm, &[]).unwrap();
        assert_eq!(decoded.restored_text, llm);
        assert!(decoded.hallucinations.is_empty());
    }

    #[test]
    fn t07_decode_no_prefix_collision_between_similar_tokens() {
        let vault = Vault::new().unwrap();
        let mappings = vec![
            map_token("Email_1", "short@example.com"),
            map_token("Email_10", "longer@example.com"),
            map_token("Email_11", "another@example.com"),
        ];
        let llm = "A [Email_1] B [Email_10] C [Email_11] D [Email_1]";

        let decoded = vault.decode(llm, &mappings).unwrap();
        assert_eq!(
            decoded.restored_text,
            "A short@example.com B longer@example.com C another@example.com D short@example.com"
        );
        assert!(decoded.hallucinations.is_empty());
    }

    #[test]
    fn t08_decode_accepts_mixed_case_tokens_from_encode() {
        let vault = Vault::new().unwrap();
        let encoded = vault
            .encode("Reach bob.jones@corp.com for details.")
            .unwrap();
        assert!(!encoded.mappings.is_empty());

        let class = encoded.mappings[0].class.clone();
        let llm = format!("Restored contact is [{}].", class);
        let decoded = vault.decode(&llm, &encoded.mappings).unwrap();

        assert!(decoded.restored_text.contains("bob.jones@corp.com"));
        assert!(decoded.hallucinations.is_empty());
    }

    #[test]
    fn t09_decode_accepts_custom_colon_tokens() {
        let vault = Vault::new().unwrap();
        let encoded = vault
            .encode("Card on file 4111-1111-1111-1111 expires soon.")
            .unwrap();
        let card_map = encoded
            .mappings
            .iter()
            .find(|m| m.value.contains("4111"))
            .unwrap_or_else(|| panic!("{:?}", encoded.mappings));
        let class = card_map.class.clone();

        let llm = format!("The card [{}] was charged.", class);
        let decoded = vault.decode(&llm, &encoded.mappings).unwrap();
        assert!(decoded.restored_text.contains("4111-1111-1111-1111"));
        assert!(decoded.hallucinations.is_empty());
    }

    #[test]
    fn t10_fixture_round_trip_preserves_non_pii_and_restores_pii() {
        let vault = Vault::new().unwrap();
        let encoded = vault.encode(FIXTURE).unwrap();

        assert_detected(&encoded, "sarah.mitchell@example.com", "email");
        assert_preserved(&encoded, "1.2.3", "version");
        assert_preserved(&encoded, "ABC-1234", "product code");

        let decoded = vault
            .decode(&encoded.redacted_text, &encoded.mappings)
            .unwrap();
        assert!(decoded.restored_text.contains("sarah.mitchell@example.com"));
        assert!(decoded.restored_text.contains("1.2.3"));
        assert!(decoded.restored_text.contains("ABC-1234"));
        assert!(decoded.hallucinations.is_empty());
    }

    // ---------- per-class detection ----------

    #[test]
    fn t11_detects_person_name_with_title() {
        let vault = Vault::new().unwrap();
        let encoded = vault
            .encode("Please contact Dr. Sarah Mitchell about the case.")
            .unwrap();

        assert_detected(&encoded, "Sarah Mitchell", "person name");
        assert!(
            encoded
                .mappings
                .iter()
                .any(|m| m.class.contains("Name") || m.class.contains("name")),
            "{:?}",
            encoded.mappings
        );
    }

    #[test]
    fn t12_detects_phone() {
        let vault = Vault::new().unwrap();
        let input = "Call her at (555) 867-5309 tomorrow.";
        let encoded = vault.encode(input).unwrap();

        assert_detected(&encoded, "5309", "phone");
        assert!(
            encoded
                .mappings
                .iter()
                .any(|m| m.class.to_lowercase().contains("phone")),
            "{:?}",
            encoded.mappings
        );

        let decoded = vault
            .decode(&encoded.redacted_text, &encoded.mappings)
            .unwrap();
        assert!(decoded.restored_text.contains("5309"));
    }

    #[test]
    fn t13_detects_ssn() {
        let vault = Vault::new().unwrap();
        let input = "The SSN on file is 123-45-6789 for payroll.";
        let encoded = vault.encode(input).unwrap();

        assert_detected(&encoded, "123-45-6789", "ssn");
        assert!(
            encoded
                .mappings
                .iter()
                .any(|m| m.class.to_lowercase().contains("ssn")),
            "{:?}",
            encoded.mappings
        );

        let decoded = vault
            .decode(&encoded.redacted_text, &encoded.mappings)
            .unwrap();
        assert!(decoded.restored_text.contains("123-45-6789"));
    }

    #[test]
    fn t14_detects_date() {
        let vault = Vault::new().unwrap();
        let input = "The meeting is scheduled for 2024-03-15. API version 1.2.3 ships same day.";
        let encoded = vault.encode(input).unwrap();

        assert_detected(&encoded, "2024-03-15", "date");
        assert!(
            encoded
                .mappings
                .iter()
                .any(|m| m.class.to_lowercase().contains("date")),
            "{:?}",
            encoded.mappings
        );
        assert_preserved(&encoded, "1.2.3", "version string");
    }

    #[test]
    fn t15_detects_location() {
        let vault = Vault::new().unwrap();
        let input = "Ship to 42 Beacon Street, Boston, MA 02108 next week.";
        let encoded = vault.encode(input).unwrap();

        assert!(
            encoded
                .mappings
                .iter()
                .any(|m| m.value.contains("Beacon") || m.value.contains("02108")),
            "{:?}",
            encoded.mappings
        );
        assert!(!encoded.redacted_text.contains("42 Beacon Street"));
        assert!(
            encoded
                .mappings
                .iter()
                .any(|m| m.class.to_lowercase().contains("location")),
            "{:?}",
            encoded.mappings
        );

        let decoded = vault
            .decode(&encoded.redacted_text, &encoded.mappings)
            .unwrap();
        assert!(decoded.restored_text.contains("Beacon"));
    }

    #[test]
    fn t16_fixture_detects_all_required_classes() {
        let vault = Vault::new().unwrap();
        let encoded = vault.encode(FIXTURE).unwrap();

        assert_detected(&encoded, "Sarah Mitchell", "person");
        assert_detected(&encoded, "5309", "phone");
        assert_detected(&encoded, "123-45-6789", "ssn");
        assert_detected(&encoded, "2024-03-15", "date");
        assert!(
            encoded
                .mappings
                .iter()
                .any(|m| m.value.contains("Beacon") || m.value.contains("02108")),
            "{:?}",
            encoded.mappings
        );
        assert_detected(&encoded, "sarah.mitchell@example.com", "email");
        assert_detected(&encoded, "4111-1111-1111-1111", "credit card");

        let decoded = vault
            .decode(&encoded.redacted_text, &encoded.mappings)
            .unwrap();
        for needle in [
            "Sarah Mitchell",
            "5309",
            "123-45-6789",
            "2024-03-15",
            "sarah.mitchell@example.com",
            "4111-1111-1111-1111",
        ] {
            assert!(decoded.restored_text.contains(needle), "{needle}");
        }
        assert!(decoded.hallucinations.is_empty());
    }

    // ---------- sample.txt regression (missed "Priya Patel") ----------

    #[test]
    fn t17_sample_detects_bare_name_after_hiring_manager_comma() {
        let vault = Vault::new().unwrap();
        let encoded = vault
            .encode(
                "If you need further details, please reach out to the hiring manager, \
Priya Patel at priya.patel@university.edu.",
            )
            .unwrap();

        assert_detected(&encoded, "Priya Patel", "bare name after role comma");
        assert_detected(&encoded, "priya.patel@university.edu", "email");
    }

    #[test]
    fn t18_sample_detects_name_after_reach_out_to_manager() {
        let vault = Vault::new().unwrap();
        let encoded = vault
            .encode("Please reach out to the hiring manager, Priya Patel for details.")
            .unwrap();

        assert_detected(&encoded, "Priya Patel", "name after reach-out cue");
    }

    #[test]
    fn t19_sample_no_false_positive_on_contact_information() {
        let vault = Vault::new().unwrap();
        let input = "Her contact information is as follows:";
        let encoded = vault.encode(input).unwrap();

        assert!(
            encoded.mappings.is_empty(),
            "false positive on common phrase 'contact information': {:?}",
            encoded.mappings
        );
        assert_eq!(encoded.redacted_text, input);
    }

    #[test]
    fn t20_sample_no_false_positive_on_team_and_role_phrases() {
        let vault = Vault::new().unwrap();
        let input = "\
To Human Resources:
Thank you,
HR Operations Team
the hiring manager will follow up
internal tracking code ABC-1234 and portal version 1.2.3
";
        let encoded = vault.encode(input).unwrap();

        assert_preserved(&encoded, "Human Resources", "org phrase");
        assert_preserved(&encoded, "HR Operations Team", "team signature");
        assert_preserved(&encoded, "ABC-1234", "product code");
        assert_preserved(&encoded, "1.2.3", "version");
        assert!(
            encoded.mappings.is_empty(),
            "false positives on non-person phrases: {:?}",
            encoded.mappings
        );
    }

    #[test]
    fn t21_sample_full_document_round_trip() {
        assert_round_trips(SAMPLE);
    }

    #[test]
    fn t22_sample_detects_every_class_including_priya() {
        let vault = Vault::new().unwrap();
        let encoded = vault.encode(SAMPLE).unwrap();

        assert_detected(&encoded, "Dr. Sarah Mitchell", "titled person");
        assert_detected(&encoded, "Priya Patel", "bare person (regression)");
        assert_detected(&encoded, "Michael Chen", "person after under-the-name");
        assert_detected(&encoded, "2026-09-24", "date");
        assert_detected(&encoded, "(555) 867-5309", "phone");
        assert_detected(&encoded, "sarah.mitchell@example.com", "email");
        assert_detected(&encoded, "priya.patel@university.edu", "second email");
        assert_detected(&encoded, "123-45-6789", "ssn");
        assert_detected(&encoded, "4111-1111-1111-1111", "credit card");
        assert!(
            encoded
                .mappings
                .iter()
                .any(|m| m.value.contains("Beacon") || m.value.contains("02108")),
            "location missing: {:?}",
            encoded.mappings
        );

        // Non-PII must survive encode.
        assert_preserved(&encoded, "Her contact information is as follows", "common phrase");
        assert_preserved(&encoded, "Human Resources", "org");
        assert_preserved(&encoded, "HR Operations Team", "signature");
        assert_preserved(&encoded, "ABC-1234", "tracking code");
        assert_preserved(&encoded, "1.2.3", "version");

        // Decode restores every PII value.
        let decoded = vault
            .decode(&encoded.redacted_text, &encoded.mappings)
            .unwrap();
        assert_eq!(decoded.restored_text, SAMPLE);
        assert!(decoded.hallucinations.is_empty());
    }

    #[test]
    fn t23_sample_decodes_llm_reply_using_mapping() {
        let vault = Vault::new().unwrap();
        let encoded = vault.encode(SAMPLE).unwrap();
        let priya = encoded
            .mappings
            .iter()
            .find(|m| m.value.contains("Priya Patel"))
            .expect("Priya Patel must be mapped");
        let token = format!("[{}]", priya.class);

        let llm = format!(
            "I contacted {token} and {token} will review the file for Dr. Sarah Mitchell's hire."
        );
        // Also include Sarah via whatever class she got.
        let sarah = encoded
            .mappings
            .iter()
            .find(|m| m.value.contains("Sarah Mitchell"))
            .expect("Sarah Mitchell must be mapped");
        let llm = llm.replace("Dr. Sarah Mitchell", &format!("[{}]", sarah.class));

        let decoded = vault.decode(&llm, &encoded.mappings).unwrap();
        assert!(
            decoded.restored_text.matches("Priya Patel").count() >= 2,
            "token not restored twice: {}",
            decoded.restored_text
        );
        assert!(decoded.restored_text.contains("Sarah Mitchell"));
        assert!(decoded.hallucinations.is_empty());
    }

    // ---------- broader name / false-positive coverage ----------

    #[test]
    fn t24_detects_middle_initial_and_multiword_names() {
        let vault = Vault::new().unwrap();
        let encoded = vault
            .encode("The account for John A. Doe was updated under the name Mary Jane Watson.")
            .unwrap();

        assert_detected(&encoded, "John A. Doe", "middle-initial name");
        assert_detected(&encoded, "Mary Jane Watson", "three-part name");
    }

    #[test]
    fn t25_detects_name_after_manager_without_title() {
        let vault = Vault::new().unwrap();
        for phrase in [
            "the hiring manager, Priya Patel",
            "the manager, Alex Kim",
            "reach out to Jordan Lee",
            "named Casey Jordan",
            "under the name Taylor Swift",
        ] {
            let encoded = vault.encode(phrase).unwrap();
            let last_word_pair = phrase
                .rsplit(&[',', ' '][..])
                .filter(|s| s.chars().next().is_some_and(|c| c.is_uppercase()))
                .take(2)
                .collect::<Vec<_>>();
            assert!(
                !encoded.mappings.is_empty(),
                "no name detected in {phrase:?} (want {last_word_pair:?}): {:?}",
                encoded.mappings
            );
        }
    }

    #[test]
    fn t26_phone_formats() {
        let vault = Vault::new().unwrap();
        for phone in [
            "(555) 867-5309",
            "555-867-5309",
            "202-555-0100",
            "(202) 555-0100",
        ] {
            let encoded = vault.encode(format!("Call {phone} now.").as_str()).unwrap();
            assert!(
                encoded.mappings.iter().any(|m| m.value.contains(&phone.replace(' ', "")))
                    || encoded.mappings.iter().any(|m| {
                        m.value.chars().filter(|c| c.is_ascii_digit()).collect::<String>()
                            == phone.chars().filter(|c| c.is_ascii_digit()).collect::<String>()
                    }),
                "phone {phone:?} not detected: {:?}",
                encoded.mappings
            );
        }
    }

    #[test]
    fn t27_ssn_formats() {
        let vault = Vault::new().unwrap();
        for line in [
            "SSN: 123-45-6789",
            "SSN 123-45-6789",
            "ssn is 123-45-6789",
            "Social Security Number: 123-45-6789",
            "her provided SSN: 123-45-6789",
        ] {
            let encoded = vault.encode(line).unwrap();
            assert_detected(&encoded, "123-45-6789", line);
        }
    }

    #[test]
    fn t28_date_formats() {
        let vault = Vault::new().unwrap();
        for date in ["2026-09-24", "2024-03-15", "03/15/2024", "3/5/24"] {
            let encoded = vault.encode(format!("Due {date} please.").as_str()).unwrap();
            assert_detected(&encoded, date, "date");
        }
    }

    #[test]
    fn t29_location_formats() {
        let vault = Vault::new().unwrap();
        for addr in [
            "42 Beacon Street, Boston, MA 02108",
            "100 Main Street",
            "Boston, MA 02108",
        ] {
            let encoded = vault.encode(format!("Ship to {addr}.").as_str()).unwrap();
            assert!(
                encoded
                    .mappings
                    .iter()
                    .any(|m| m.class.to_lowercase().contains("location")),
                "location {addr:?} not detected: {:?}",
                encoded.mappings
            );
            assert!(
                !encoded.redacted_text.contains(addr) || addr.contains("Boston, MA") && {
                    // city/state alone may appear inside a larger redacted span
                    !encoded.redacted_text.contains("42 Beacon")
                },
                "address leaked: {}",
                encoded.redacted_text
            );
        }
    }

    #[test]
    fn t30_empty_and_plain_text_unchanged() {
        let vault = Vault::new().unwrap();
        for input in ["", "   ", "Hello world, nothing sensitive here."] {
            let encoded = vault.encode(input).unwrap();
            assert!(encoded.mappings.is_empty(), "{input:?} -> {:?}", encoded.mappings);
            let decoded = vault
                .decode(&encoded.redacted_text, &encoded.mappings)
                .unwrap();
            assert_eq!(decoded.restored_text, input);
        }
    }

    #[test]
    fn t31_decode_does_not_invent_values_for_unknown_tokens() {
        let vault = Vault::new().unwrap();
        let mappings = vec![map_token("Name_1", "Alice")];
        let llm = "Ask [Name_1] or [Name_2] or [Custom:ssn_1].";

        let decoded = vault.decode(llm, &mappings).unwrap();
        assert!(decoded.restored_text.contains("Alice"));
        assert!(decoded.restored_text.contains("[Name_2]"));
        assert!(decoded.restored_text.contains("[Custom:ssn_1]"));
        assert_eq!(decoded.hallucinations.len(), 2);
    }

    #[test]
    fn t32_repeated_same_value_reuses_consistent_mapping() {
        let vault = Vault::new().unwrap();
        let encoded = vault
            .encode("Email bob@x.com and also bob@x.com again.")
            .unwrap();

        let bob_maps: Vec<_> = encoded
            .mappings
            .iter()
            .filter(|m| m.value == "bob@x.com")
            .collect();
        assert!(!bob_maps.is_empty());
        // encode maps the internal gaze token once; redacted text must not leak.
        assert!(!encoded.redacted_text.contains("bob@x.com"));
    }

    #[test]
    fn t33_llm_parity_with_sample_tokens() {
        // Simulate an LLM that copies tokens verbatim from the redacted sample.
        let vault = Vault::new().unwrap();
        let encoded = vault.encode(SAMPLE).unwrap();
        let decoded = vault
            .decode(&encoded.redacted_text, &encoded.mappings)
            .unwrap();
        assert_eq!(decoded.restored_text, SAMPLE);
    }

    // ---------- BUG fixes ----------

    /// BUG 1 — source `<email>` must not leak `<<session_hex:Email_N>>`.
    #[test]
    fn bug1_angle_wrapped_email_renders_as_readable_token() {
        let vault = Vault::new().unwrap();
        let encoded = vault
            .encode("Contact <s.mitchell@university.edu> directly")
            .unwrap();

        assert_eq!(
            encoded.redacted_text,
            "Contact [Email_1] directly",
            "mappings: {:?}",
            encoded.mappings
        );
        assert!(
            !encoded.redacted_text.contains('<') || encoded.redacted_text.contains("[Email_1]"),
            "session prefix leaked: {}",
            encoded.redacted_text
        );
        assert!(
            !encoded.redacted_text.contains("Email_4>") && !encoded.redacted_text.contains(">>"),
            "raw gaze token leaked: {}",
            encoded.redacted_text
        );
    }

    /// BUG 2 — repeated surface form must share one session token (single call + across calls).
    #[test]
    fn bug2_session_accumulates_same_value_across_mentions() {
        let vault = Vault::new().unwrap();
        let encoded = vault
            .encode("Dr. Sarah Mitchell met with Sarah Mitchell yesterday.")
            .unwrap();

        assert!(
            encoded.redacted_text.contains("[Name_1]"),
            "expected [Name_1] in {:?} (mappings {:?})",
            encoded.redacted_text,
            encoded.mappings
        );
        assert_eq!(
            encoded.redacted_text.matches("[Name_1]").count(),
            2,
            "session did not accumulate; each mention got a fresh token: {} {:?}",
            encoded.redacted_text,
            encoded.mappings
        );
        assert!(
            !encoded.redacted_text.contains("Sarah Mitchell"),
            "second mention leaked: {}",
            encoded.redacted_text
        );

        // Across encode() calls the same session must keep the same token.
        let again = vault.encode("Dr. Sarah Mitchell called back.").unwrap();
        assert_eq!(
            again.redacted_text,
            "[Name_1] called back.",
            "session token not reused across calls: {:?}",
            again.mappings
        );
    }

    /// BUG 2 — one person must not split across Name_1 / Name_2 for one encode.
    #[test]
    fn bug2_user_sentence_shares_one_token_for_sarah_mitchell() {
        let vault = Vault::new().unwrap();
        let encoded = vault
            .encode("Dr. Sarah Mitchell met with Sarah Mitchell yesterday.")
            .unwrap();

        let sarah_classes: Vec<_> = encoded
            .mappings
            .iter()
            .filter(|m| m.value.contains("Sarah Mitchell"))
            .map(|m| m.class.clone())
            .collect();
        let mut unique = sarah_classes.clone();
        unique.sort();
        unique.dedup();
        assert!(
            unique.len() <= 1,
            "one person split across tokens (session not accumulating): {:?}",
            encoded.mappings
        );
    }

    /// BUG 3 — E.164 international / German landline phones; dates and refs stay clean.
    #[test]
    fn bug3_detects_e164_international_phones() {
        let vault = Vault::new().unwrap();
        for phone in [
            "+49 30 1234 5678",
            "+61 412 345 678",
            "+1-617-555-0198",
            "+86 138 0013 8000",
        ] {
            let encoded = vault.encode(format!("Reach us at {phone} today.").as_str()).unwrap();
            let digits = |s: &str| {
                s.chars()
                    .filter(|c| c.is_ascii_digit())
                    .collect::<String>()
            };
            assert!(
                encoded.mappings.iter().any(|m| digits(&m.value) == digits(phone)
                    || m.value.contains(phone)),
                "phone {phone:?} not detected: {:?}",
                encoded.mappings
            );
            assert!(
                !encoded.redacted_text.contains(phone),
                "phone {phone:?} leaked: {}",
                encoded.redacted_text
            );
        }
    }

    #[test]
    fn bug3_does_not_catch_dates_versions_or_refs() {
        let vault = Vault::new().unwrap();
        for noise in [
            "2026-09-24",
            "1.2.3",
            "INV-2026-0918",
            "ABC-1234",
            "$145,000",
        ] {
            let encoded = vault
                .encode(format!("Tracking value {noise} must stay readable.").as_str())
                .unwrap();
            assert!(
                !encoded.mappings.iter().any(|m| m
                    .class
                    .to_lowercase()
                    .contains("phone")),
                "phone false positive on {noise:?}: {:?}",
                encoded.mappings
            );
            // Non-date noise must remain literal (dates may legitimately redact as date).
            if noise != "2026-09-24" {
                assert!(
                    encoded.redacted_text.contains(noise),
                    "false positive on {noise:?}: {}",
                    encoded.redacted_text
                );
            } else {
                assert!(
                    !encoded.redacted_text.contains("2026-09-24")
                        || encoded
                            .mappings
                            .iter()
                            .any(|m| m.class.to_lowercase().contains("date")),
                    "date should redact as date, not leak as phone: {}",
                    encoded.redacted_text
                );
            }
        }
    }

    // ---------- CHANGE 1 — session prefix / complex family tokens ----------

    #[test]
    fn change1_family_iban_token_becomes_readable() {
        assert_eq!(
            readable_token("<680311de:Custom:family:payment-card-or-iban_1>"),
            "[Iban_1]"
        );
    }

    #[test]
    fn change1_simple_email_token_unchanged() {
        assert_eq!(readable_token("<422c7695:Email_4>"), "[Email_4]");
    }

    #[test]
    fn change1_custom_date_token_capitalises() {
        assert_eq!(readable_token("<422c7695:Custom:date_1>"), "[Date_1]");
    }

    // ---------- CHANGE 2 — mapping display order by first appearance ----------

    #[test]
    fn change2_mappings_sorted_by_first_appearance_in_source() {
        let vault = Vault::new().unwrap();
        // Email appears after the phone number in the source.
        let input = "Call (555) 867-5309 then email zoe.last@example.com.";
        let encoded = vault.encode(input).unwrap();

        assert!(
            encoded.mappings.len() >= 2,
            "need ≥2 mappings: {:?}",
            encoded.mappings
        );
        let phone_idx = input.find("5309").unwrap();
        let email_idx = input.find("zoe.last@example.com").unwrap();
        let phone_pos = encoded
            .mappings
            .iter()
            .position(|m| m.value.contains("5309"))
            .expect("phone mapping");
        let email_pos = encoded
            .mappings
            .iter()
            .position(|m| m.value.contains("zoe.last@example.com"))
            .expect("email mapping");

        assert!(
            phone_pos < email_pos,
            "mappings must follow source order (phone@{phone_idx} before email@{email_idx}): {:?}",
            encoded.mappings
        );
        assert!(
            encoded.mappings[phone_pos].first_offset <= encoded.mappings[email_pos].first_offset
        );
    }

    #[test]
    fn change2_mapping_sort_is_display_order_only() {
        let vault = Vault::new().unwrap();
        let input = "Call (555) 867-5309 then email zoe.last@example.com.";
        let encoded = vault.encode(input).unwrap();
        let decoded = vault
            .decode(&encoded.redacted_text, &encoded.mappings)
            .unwrap();
        assert_eq!(decoded.restored_text, input);
    }
}
