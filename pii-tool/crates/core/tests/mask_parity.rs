//! Behavioural parity with laroccacharly/mask (consultant / gliner / reject / token tests).
//! Mask uses GLiNER + llama.cpp for bare names; we assert the same product outcomes
//! against our vault, using the name-cue phrases our detectors support.

use pii_core::vault::{TokenMapping, Vault};

const PERSON: &str = "Priya Nair";
const FOLDED_PERSON: &str = "priya nair";

// mask/tests/consultant.rs fixtures, with the hiring-manager / title cues our NER needs
// (Mask's GLiNER catches bare and ALL-CAPS names without cues — known gap on our side).
const INTERVIEW_NOTES: &str = "\
Northwind Logistics — discovery interview
Date: 12 March 2026
Interviewer: Jonah Hale, Aperture Consulting
Client attendee: hiring manager, Dr. Priya Nair, Director of Operations, Northwind Logistics

The hiring manager, Priya Nair said warehouse overtime in Cleveland is the main cost driver.
She asked Aperture for a six-week diagnostic with a steering readout on 24 April 2026.
You must send the Affiliate a copy of the readout. Do not email the customer.
";

const KICKOFF_MEMO: &str = "\
Internal kickoff memo
From: Elena Voss, Aperture Consulting
Re: Northwind Logistics diagnostic

The hiring manager, Priya Nair is the day-to-day client owner for this engagement.
Weekly steering calls include Dr. Priya Nair; do not copy other Northwind staff until she approves.
";

/// Mask `src/reject.rs` contract/role words that must never be tokenized.
const REJECTED: &[&str] = &["You", "Affiliate", "customer", "Director", "person", "employee", "email", "link"];

fn labels_in(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            if let Some(close) = text[i..].find(']') {
                found.push(text[i + 1..i + close].to_string());
                i += close + 1;
                continue;
            }
        }
        i += 1;
    }
    found
}

fn shared_labels(left: &str, right: &str) -> Vec<String> {
    let right_labels = labels_in(right);
    labels_in(left)
        .into_iter()
        .filter(|label| right_labels.contains(label))
        .collect()
}

fn value_of<'a>(mappings: &'a [TokenMapping], needle: &str) -> Option<&'a TokenMapping> {
    mappings.iter().find(|m| m.value.contains(needle))
}

// ---------- mask/tests/consultant.rs ----------

#[test]
fn shared_person_keeps_one_label_across_documents_and_llm_roundtrip() {
    let vault = Vault::new().expect("vault");

    let notes = vault.encode(INTERVIEW_NOTES).expect("encode notes");
    let memo = vault.encode(KICKOFF_MEMO).expect("encode memo");

    assert!(
        !notes.redacted_text.contains(PERSON),
        "notes leaked {PERSON}: {}",
        notes.redacted_text
    );
    assert!(
        !memo.redacted_text.contains(PERSON),
        "memo leaked {PERSON}: {}",
        memo.redacted_text
    );

    for word in REJECTED {
        let in_notes = notes.redacted_text.contains(word);
        let in_memo = memo.redacted_text.contains(word);
        if in_notes || in_memo {
            continue;
        }
        // Only require words that appear in the source fixtures.
        let in_source = INTERVIEW_NOTES.contains(word) || KICKOFF_MEMO.contains(word);
        assert!(!in_source || in_notes || in_memo, "rejected word {word:?} stripped");
    }
    assert!(
        notes.redacted_text.contains("You")
            && notes.redacted_text.contains("Affiliate")
            && notes.redacted_text.contains("Director")
            && notes.redacted_text.contains("customer"),
        "rejected contract words were tokenized: {}",
        notes.redacted_text
    );

    let shared = shared_labels(&notes.redacted_text, &memo.redacted_text);
    assert!(
        !shared.is_empty(),
        "expected {PERSON} to share one label across docs\nnotes: {}\nmemo: {}",
        notes.redacted_text,
        memo.redacted_text
    );

    let owner_line = memo
        .redacted_text
        .lines()
        .find(|line| line.contains("day-to-day"))
        .expect("owner line");
    let calls_line = memo
        .redacted_text
        .lines()
        .find(|line| line.contains("Weekly steering"))
        .expect("calls line");
    assert!(
        labels_in(owner_line)
            .iter()
            .any(|label| calls_line.contains(&format!("[{label}]"))),
        "expected titled and bare forms of {PERSON} to share one label\nmemo: {}",
        memo.redacted_text
    );

    // Mask runs llama.cpp here; we simulate a compliant LLM reply that copies tokens.
    let label = shared[0].clone();
    let reply = format!("The Director of Operations is [{label}].");
    assert!(!reply.contains(PERSON), "simulated reply leaked real name");

    let decoded = vault.decode(&reply, &memo.mappings).expect("decode");
    assert!(
        decoded
            .restored_text
            .to_lowercase()
            .contains(FOLDED_PERSON),
        "decoded reply should restore {FOLDED_PERSON}, got {}",
        decoded.restored_text
    );
}

// ---------- mask/tests/gliner.rs (entity classes, our detectors) ----------

#[test]
fn detects_named_entities_like_gliner_labels() {
    let vault = Vault::new().expect("vault");

    let agreement = "This Agreement is made on 2024-03-15, by hiring manager, \
Dr. Marie Tremblay, VP of Engineering at Acme Robotics Corp in Montreal, Canada. \
Shipment address: 42 Beacon Street, Boston, MA 02108. \
Contact: marie.tremblay@acme-robotics.example";
    let encoded = vault.encode(agreement).expect("encode agreement");

    assert!(
        value_of(&encoded.mappings, "Marie Tremblay").is_some(),
        "expected person Marie Tremblay, got {:?}",
        encoded.mappings
    );
    assert!(
        value_of(&encoded.mappings, "marie.tremblay@acme-robotics.example").is_some(),
        "expected email, got {:?}",
        encoded.mappings
    );
    assert!(
        encoded
            .mappings
            .iter()
            .any(|m| m.value.contains("Beacon") || m.value.contains("02108") || m.value.contains("Boston")),
        "expected location, got {:?}",
        encoded.mappings
    );
    assert!(
        encoded
            .mappings
            .iter()
            .any(|m| m.value.contains("2024-03-15") || m.class.to_lowercase().contains("date")),
        "expected date, got {:?}",
        encoded.mappings
    );

    let notice = "Notices to the Affiliate shall be sent to the hiring manager, \
Dr. Giuseppe Verdi at g.verdi@example.it, Milano.";
    let encoded = vault.encode(notice).expect("encode notice");
    assert!(
        value_of(&encoded.mappings, "Giuseppe Verdi").is_some(),
        "expected person Giuseppe Verdi, got {:?}",
        encoded.mappings
    );
    assert!(
        value_of(&encoded.mappings, "g.verdi@example.it").is_some(),
        "expected email g.verdi@example.it, got {:?}",
        encoded.mappings
    );
    assert!(
        encoded.redacted_text.contains("Affiliate"),
        "Affiliate rejected word stripped: {}",
        encoded.redacted_text
    );
}

// ---------- mask/src/token_formatter.rs + gaze preserve-token unit test ----------

#[test]
fn compact_tokens_round_trip() {
    let vault = Vault::new().expect("vault");
    let encoded = vault
        .encode("Email priya.nair@example.com about the readout.")
        .expect("encode");

    let label = labels_in(&encoded.redacted_text)
        .into_iter()
        .next()
        .expect("one token");
    assert!(
        label.contains('_'),
        "compact token should look like Class_N, got {label}"
    );
    assert!(
        encoded.redacted_text.contains(&format!("[{label}]")),
        "readable [Class_N] form missing: {}",
        encoded.redacted_text
    );

    let decoded = vault
        .decode(&encoded.redacted_text, &encoded.mappings)
        .expect("decode");
    assert!(
        decoded.restored_text.contains("priya.nair@example.com"),
        "round-trip lost value: {}",
        decoded.restored_text
    );
}

#[test]
fn pipeline_preserves_existing_token_and_tokenizes_remaining_pii() {
    let vault = Vault::new().expect("vault");
    let first = vault
        .encode("Write to Dr. Priya Nair at priya.nair@example.com")
        .expect("encode name+email");
    let name_map = value_of(&first.mappings, "Priya Nair").expect("name mapping");
    let existing = format!("[{}]", name_map.class);

    let second = vault
        .encode(&format!("{existing} emailed other.person@example.com"))
        .expect("encode with existing token");

    assert!(
        second.redacted_text.contains(&existing),
        "existing token {existing} changed: {}",
        second.redacted_text
    );
    let other = value_of(&second.mappings, "other.person@example.com");
    assert!(
        other.is_some() || !second.redacted_text.contains("other.person@example.com"),
        "email was not tokenized: {}",
        second.redacted_text
    );
}

// ---------- reject-word false positives (mask/src/reject.rs intent) ----------

#[test]
fn reject_contract_words_are_not_tokenized() {
    let vault = Vault::new().expect("vault");
    let input = "You must send the Affiliate and the customer a copy. \
The Director and employee must not share the link on the website page.";
    let encoded = vault.encode(input).expect("encode");

    for word in ["You", "Affiliate", "customer", "Director", "employee", "link"] {
        assert!(
            encoded.redacted_text.contains(word),
            "rejected word {word:?} was tokenized: {}",
            encoded.redacted_text
        );
    }
    assert!(
        encoded.mappings.is_empty(),
        "false positives on contract words: {:?}",
        encoded.mappings
    );
}
