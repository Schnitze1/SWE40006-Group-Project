//! Span-level gold metrics: recall, precision, category (folded), exact category, traps.
//!
//! Test-only. Does not change detection/extraction/vault/rules.
//!
//! Metrics:
//! - Category  — normalise_class(raw class) vs normalise_class(expected_class)
//! - Exact     — exact_key(raw token class) vs expected_exact_class
//!               (e.g. Email_1 → EMAIL, Credit_card_1 → CREDIT_CARD, Ssn_1 → SSN)
//!
//! Run: cargo test -p pii-core --test gold_suite -- --nocapture
//! Report: pii-tool/test_data/gold_report.md

use std::fmt::Write as _;
use std::path::PathBuf;

use pii_core::extract::extract_text;
use pii_core::vault::{EncodedOutput, Vault};

struct GoldEntity {
    value: &'static str,
    /// Folded family (PERSON, EMAIL, …) — used by normalise_class comparison.
    expected_class: &'static str,
    /// Precise real-world type (PASSPORT, TFN, SWIFT, …) — exact-category metric.
    expected_exact_class: &'static str,
    category_group: &'static str,
}

struct Trap {
    value: &'static str,
    reason: &'static str,
}

const GOLD: &[GoldEntity] = &[
    // PERSON (8)
    GoldEntity { value: "Sarah Elizabeth Mitchell", expected_class: "PERSON", expected_exact_class: "PERSON", category_group: "PII" },
    GoldEntity { value: "Sarah E. Mitchell", expected_class: "PERSON", expected_exact_class: "PERSON", category_group: "PII" },
    GoldEntity { value: "Sarah Mitchell", expected_class: "PERSON", expected_exact_class: "PERSON", category_group: "PII" },
    GoldEntity { value: "Priya Patel", expected_class: "PERSON", expected_exact_class: "PERSON", category_group: "PII" },
    GoldEntity { value: "Michael Chen", expected_class: "PERSON", expected_exact_class: "PERSON", category_group: "PII" },
    GoldEntity { value: "James O'Brien-Smith", expected_class: "PERSON", expected_exact_class: "PERSON", category_group: "PII" },
    GoldEntity { value: "José Müller", expected_class: "PERSON", expected_exact_class: "PERSON", category_group: "PII" },
    GoldEntity { value: "张伟", expected_class: "PERSON", expected_exact_class: "PERSON", category_group: "PII" },
    // DATE (6)
    GoldEntity { value: "14/03/1988", expected_class: "DATE", expected_exact_class: "DATE", category_group: "PII" },
    GoldEntity { value: "12 Nov 2031", expected_class: "DATE", expected_exact_class: "DATE", category_group: "PII" },
    GoldEntity { value: "2026-09-24", expected_class: "DATE", expected_exact_class: "DATE", category_group: "PII" },
    GoldEntity { value: "12 Oct 2026", expected_class: "DATE", expected_exact_class: "DATE", category_group: "PII" },
    GoldEntity { value: "22 Sept 2026", expected_class: "DATE", expected_exact_class: "DATE", category_group: "PII" },
    GoldEntity { value: "24 September 2026", expected_class: "DATE", expected_exact_class: "DATE", category_group: "PII" },
    // PHONE (6)
    GoldEntity { value: "+61 412 345 678", expected_class: "PHONE", expected_exact_class: "PHONE", category_group: "PII" },
    GoldEntity { value: "(02) 9876 5432", expected_class: "PHONE", expected_exact_class: "PHONE", category_group: "PII" },
    GoldEntity { value: "+1-617-555-0198", expected_class: "PHONE", expected_exact_class: "PHONE", category_group: "PII" },
    GoldEntity { value: "617.555.0142", expected_class: "PHONE", expected_exact_class: "PHONE", category_group: "PII" },
    GoldEntity { value: "+49 30 1234 5678", expected_class: "PHONE", expected_exact_class: "PHONE", category_group: "PII" },
    GoldEntity { value: "+86 138 0013 8000", expected_class: "PHONE", expected_exact_class: "PHONE", category_group: "PII" },
    // EMAIL (7)
    GoldEntity { value: "sarah.mitchell+onboarding@example.com", expected_class: "EMAIL", expected_exact_class: "EMAIL", category_group: "PII" },
    GoldEntity { value: "s.mitchell@university.edu", expected_class: "EMAIL", expected_exact_class: "EMAIL", category_group: "PII" },
    GoldEntity { value: "priya.patel@university.edu", expected_class: "EMAIL", expected_exact_class: "EMAIL", category_group: "PII" },
    GoldEntity { value: "m.chen@personal-domain.io", expected_class: "EMAIL", expected_exact_class: "EMAIL", category_group: "PII" },
    GoldEntity { value: "j.obrien-smith@unimelb.edu.au", expected_class: "EMAIL", expected_exact_class: "EMAIL", category_group: "PII" },
    GoldEntity { value: "jose.muller@firma.de", expected_class: "EMAIL", expected_exact_class: "EMAIL", category_group: "PII" },
    GoldEntity { value: "zhang.wei@example.cn", expected_class: "EMAIL", expected_exact_class: "EMAIL", category_group: "PII" },
    // IDENTITY (6)
    GoldEntity { value: "PA1234567", expected_class: "PASSPORT", expected_exact_class: "PASSPORT", category_group: "PII" },
    GoldEntity { value: "123-45-6789", expected_class: "SSN", expected_exact_class: "SSN", category_group: "PII" },
    GoldEntity { value: "123 456 789", expected_class: "TFN", expected_exact_class: "TFN", category_group: "PII" },
    GoldEntity { value: "2123 45670 1", expected_class: "MEDICARE", expected_exact_class: "MEDICARE", category_group: "PII" },
    GoldEntity { value: "S12345678", expected_class: "DRIVERS_LICENCE", expected_exact_class: "DRIVERS_LICENCE", category_group: "PII" },
    GoldEntity { value: "EMP-0098421", expected_class: "EMPLOYEE_ID", expected_exact_class: "EMPLOYEE_ID", category_group: "PII" },
    // FINANCIAL (4 + 3 cards)
    GoldEntity { value: "062-000", expected_class: "PAYROLL", expected_exact_class: "PAYROLL", category_group: "FINANCIAL" },
    GoldEntity { value: "1234 5678", expected_class: "PAYROLL", expected_exact_class: "PAYROLL", category_group: "FINANCIAL" },
    GoldEntity { value: "GB29 NWBK 6016 1331 9268 19", expected_class: "IBAN", expected_exact_class: "IBAN", category_group: "FINANCIAL" },
    GoldEntity { value: "NWBKGB2L", expected_class: "SWIFT", expected_exact_class: "SWIFT", category_group: "FINANCIAL" },
    GoldEntity { value: "4111-1111-1111-1111", expected_class: "CREDIT_CARD", expected_exact_class: "CREDIT_CARD", category_group: "FINANCIAL" },
    GoldEntity { value: "3782 822463 10005", expected_class: "CREDIT_CARD", expected_exact_class: "CREDIT_CARD", category_group: "FINANCIAL" },
    GoldEntity { value: "5555 5555 5555 4444", expected_class: "CREDIT_CARD", expected_exact_class: "CREDIT_CARD", category_group: "FINANCIAL" },
    // LOCATION (8)
    GoldEntity { value: "42 Beacon Street", expected_class: "LOCATION", expected_exact_class: "LOCATION", category_group: "PII" },
    GoldEntity { value: "Boston, MA 02108", expected_class: "LOCATION", expected_exact_class: "LOCATION", category_group: "PII" },
    GoldEntity { value: "Unit 7/15 Harbour Esplanade", expected_class: "LOCATION", expected_exact_class: "LOCATION", category_group: "PII" },
    GoldEntity { value: "Docklands VIC 3008", expected_class: "LOCATION", expected_exact_class: "LOCATION", category_group: "PII" },
    GoldEntity { value: "100 Cambridge Street", expected_class: "LOCATION", expected_exact_class: "LOCATION", category_group: "PII" },
    GoldEntity { value: "Boston MA 02114", expected_class: "LOCATION", expected_exact_class: "LOCATION", category_group: "PII" },
    GoldEntity { value: "Parkville VIC 3010", expected_class: "LOCATION", expected_exact_class: "LOCATION", category_group: "PII" },
    GoldEntity { value: "1 Parliament Square", expected_class: "LOCATION", expected_exact_class: "LOCATION", category_group: "PII" },
    // NETWORK (4) — expected_exact_class is IP per taxonomy spec
    GoldEntity { value: "10.0.14.22", expected_class: "NETWORK", expected_exact_class: "IP", category_group: "NETWORK" },
    GoldEntity { value: "10.0.14.23", expected_class: "NETWORK", expected_exact_class: "IP", category_group: "NETWORK" },
    GoldEntity { value: "10.0.14.0/24", expected_class: "NETWORK", expected_exact_class: "IP", category_group: "NETWORK" },
    GoldEntity {
        value: "2001:0db8:85a3::8a2e:0370:7334",
        expected_class: "NETWORK",
        expected_exact_class: "IP",
        category_group: "NETWORK",
    },
    // CRYPTO (2)
    GoldEntity {
        value: "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh",
        expected_class: "CRYPTO",
        expected_exact_class: "CRYPTO",
        category_group: "FINANCIAL",
    },
    GoldEntity {
        value: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e",
        expected_class: "CRYPTO",
        expected_exact_class: "CRYPTO",
        category_group: "FINANCIAL",
    },
];

const TRAPS: &[Trap] = &[
    Trap { value: "ABC-1234", reason: "product code" },
    Trap { value: "1.2.3", reason: "version number" },
    Trap { value: "v2026.09.24-rc1", reason: "deployment tag" },
    Trap { value: "INV-2026-0918", reason: "invoice number" },
    Trap { value: "PO-88-4412", reason: "purchase order" },
    Trap { value: "CASE-2026-0042", reason: "case reference" },
    Trap { value: "JIRA-9821", reason: "ticket id" },
    Trap { value: "QF401", reason: "flight number" },
    Trap { value: "MSFT", reason: "ticker" },
    Trap { value: "BHP", reason: "ticker" },
    Trap { value: "CBA", reason: "ticker" },
    Trap { value: "Microsoft Corporation", reason: "public company" },
    Trap { value: "BHP Group Limited", reason: "public company" },
    Trap { value: "Commonwealth Bank", reason: "public company" },
    Trap { value: "Senior Software Engineer", reason: "job title" },
    Trap { value: "HR Business Partner", reason: "job title" },
    Trap { value: "Financial Analyst", reason: "job title" },
];

/// Fold internal gaze classes and gold expected_class into one taxonomy (Category metric).
fn normalise_class(raw: &str) -> &'static str {
    let s = raw.to_lowercase();
    match s.as_str() {
        s if s.contains("ssn")
            || s.contains("passport")
            || s.contains("tfn")
            || s.contains("medicare")
            || s.contains("licence")
            || s.contains("license")
            || s.contains("dln")
            || s.contains("employee")
            || s.contains("identity") =>
        {
            "IDENTITY"
        }
        s if s.contains("iban")
            || s.contains("swift")
            || s.contains("payroll")
            || s.contains("credit")
            || s.contains("crypto")
            || s.contains("btc")
            || s.contains("eth")
            || s.contains("wallet")
            || s.contains("financial") =>
        {
            "FINANCIAL"
        }
        s if s.contains("email") || s.contains("phone") || s.contains("contact") => "CONTACT",
        s if s.contains("name") || s.contains("person") => "PERSON",
        s if s.contains("location") => "LOCATION",
        s if s.contains("ip") || s.contains("network") => "NETWORK",
        s if s.contains("date") => "DATE",
        _ => "OTHER",
    }
}

/// Extract the type from a raw/display token class (e.g. "Email_1" → "EMAIL").
/// Exact-category compares this against `expected_exact_class` (case-insensitive).
///
/// Steps: lowercase, strip trailing `_N`, `-'` → `_`, drop display `address`/`v4`/`v6`
/// wrappers so `Ip_address_1` → `IP`, `Eth_address_1` → `CRYPTO`.
fn exact_key(raw: &str) -> String {
    let mut s = raw.to_lowercase().replace('-', "_").replace(' ', "_");
    // strip trailing _N / _NN counter
    while s.len() > 2 {
        let last = s.rsplit('_').next().unwrap_or("");
        if !last.is_empty() && last.chars().all(|c| c.is_ascii_digit()) {
            let cut = s.len() - last.len() - 1;
            s.truncate(cut);
        } else {
            break;
        }
    }
    // display wrappers used by gaze class names
    for suffix in ["_address", "_v4", "_v6", "_number"] {
        if let Some(stripped) = s.strip_suffix(suffix) {
            if !stripped.is_empty() {
                s = stripped.to_string();
            }
        }
    }
    if s == "eth" || s == "btc" || s == "wallet" || s == "eth_address" {
        return "CRYPTO".to_string();
    }
    // Display class "Name_*" is the PERSON type in the gold taxonomy.
    if s == "name" {
        return "PERSON".to_string();
    }
    // Readable short forms from custom:dln / custom:employee.
    if s == "dln" {
        return "DRIVERS_LICENCE".to_string();
    }
    if s == "employee" {
        return "EMPLOYEE_ID".to_string();
    }
    s.to_uppercase()
}

fn gold_total() -> usize {
    GOLD.len()
}

fn match_gold(value: &str) -> Option<&GoldEntity> {
    GOLD.iter()
        .find(|g| value.contains(g.value) || (g.value.contains(value) && value.len() >= 6))
}

fn context_around(redacted: &str, token: &str) -> String {
    if token.is_empty() {
        return String::new();
    }
    match redacted.find(token) {
        Some(pos) => {
            let start = floor_char_boundary(redacted, pos.saturating_sub(10));
            let end = ceil_char_boundary(redacted, (pos + token.len() + 10).min(redacted.len()));
            format!("...{}...", &redacted[start..end])
        }
        None => String::new(),
    }
}

fn floor_char_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_char_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

struct PathScore {
    label: String,
    recall_hit: usize,
    recall_total: usize,
    missed: Vec<String>,
    precision_hit: usize,
    precision_total: usize,
    false_positives: Vec<String>,
    category_hit: usize,
    category_total: usize,
    category_mismatches: Vec<String>,
    exact_hit: usize,
    exact_total: usize,
    exact_mismatches: Vec<String>,
    traps_leaked: Vec<String>,
}

fn score_path(label: &str, source: &str) -> PathScore {
    let mut score = PathScore {
        label: label.to_string(),
        recall_hit: 0,
        recall_total: gold_total(),
        missed: Vec::new(),
        precision_hit: 0,
        precision_total: 0,
        false_positives: Vec::new(),
        category_hit: 0,
        category_total: 0,
        category_mismatches: Vec::new(),
        exact_hit: 0,
        exact_total: 0,
        exact_mismatches: Vec::new(),
        traps_leaked: Vec::new(),
    };

    let encoded: EncodedOutput = match Vault::new() {
        Ok(v) => match v.encode(source) {
            Ok(e) => e,
            Err(_) => {
                for g in GOLD {
                    score
                        .missed
                        .push(format!("{} ({}) [encode failed]", g.value, g.expected_class));
                }
                return score;
            }
        },
        Err(_) => {
            for g in GOLD {
                score
                    .missed
                    .push(format!("{} ({}) [vault failed]", g.value, g.expected_class));
            }
            return score;
        }
    };
    let redacted = encoded.redacted_text.as_str();
    let mappings = &encoded.mappings;

    for g in GOLD {
        let in_source = source.contains(g.value);
        let leaked = redacted.contains(g.value);
        if !in_source {
            score.missed.push(format!(
                "{} ({}/{}) [not in extract]",
                g.value, g.expected_class, g.category_group
            ));
            continue;
        }
        if !leaked {
            score.recall_hit += 1;
        } else {
            score.missed.push(format!(
                "{} ({}/{})",
                g.value, g.expected_class, g.category_group
            ));
        }
    }

    score.precision_total = mappings.len();
    // Session-collision check: two different values must not share one token/class id.
    let mut class_owners: Vec<(String, String)> = Vec::new(); // (class, value)
    for m in mappings {
        for (cls, val) in &class_owners {
            if *cls == m.class && *val != m.value {
                score.false_positives.push(format!(
                    "WARNING session collision: class \"{}\" used for both \"{}\" and \"{}\"",
                    m.class, val, m.value
                ));
            }
        }
        class_owners.push((m.class.clone(), m.value.clone()));

        match match_gold(&m.value) {
            Some(g) => {
                score.precision_hit += 1;
                let expected_n = normalise_class(g.expected_class);
                let actual_n = normalise_class(&m.class);
                score.category_total += 1;
                if expected_n == actual_n {
                    score.category_hit += 1;
                } else {
                    score.category_mismatches.push(format!(
                        "\"{}\"  expected {}  got {}",
                        g.value, expected_n, actual_n
                    ));
                }

                // Exact category: raw token class (Email_1 → EMAIL) vs expected_exact_class.
                score.exact_total += 1;
                let raw_key = exact_key(&m.class);
                let expected_key = g.expected_exact_class.to_uppercase().replace('-', "_");
                if raw_key == expected_key {
                    score.exact_hit += 1;
                } else {
                    score.exact_mismatches.push(format!(
                        "\"{}\"   raw {:<16} folded {:<10} expected {}",
                        g.value, m.class, actual_n, g.expected_exact_class
                    ));
                }
            }
            None => {
                let token = format!("[{}]", m.class);
                let ctx = context_around(redacted, &token);
                score
                    .false_positives
                    .push(format!("{} -> \"{}\"    context: \"{}\"", token, m.value, ctx));
            }
        }
    }

    for t in TRAPS {
        if source.contains(t.value) && !redacted.contains(t.value) {
            let culprit = mappings
                .iter()
                .find(|m| m.value.contains(t.value) || t.value.contains(m.value.as_str()))
                .map(|m| format!("[{}] as \"{}\"", m.class, m.value))
                .unwrap_or_else(|| "unknown token".to_string());
            score
                .traps_leaked
                .push(format!("{} ({}) replaced by {}", t.value, t.reason, culprit));
        }
    }

    score
}

fn pct(num: usize, den: usize) -> f64 {
    if den == 0 {
        100.0
    } else {
        100.0 * num as f64 / den as f64
    }
}

fn render_mismatches(out: &mut String, rows: &[String], indent: &str) {
    const MAX: usize = 20;
    for row in rows.iter().take(MAX) {
        let _ = writeln!(out, "{}{}", indent, row);
    }
    if rows.len() > MAX {
        let _ = writeln!(out, "{}… and {} more", indent, rows.len() - MAX);
    }
}

fn render_path(s: &PathScore, out: &mut String) {
    let _ = writeln!(out, "=== {} ===", s.label);
    let _ = writeln!(
        out,
        "Recall:    {}/{}  ({:.1}%)   missed: {}",
        s.recall_hit,
        s.recall_total,
        pct(s.recall_hit, s.recall_total),
        if s.missed.is_empty() {
            "none".to_string()
        } else {
            format!("[{}]", s.missed.join("; "))
        }
    );
    let _ = writeln!(
        out,
        "Precision: {}/{}  ({:.1}%)   false positives: {}",
        s.precision_hit,
        s.precision_total,
        pct(s.precision_hit, s.precision_total),
        if s.false_positives.is_empty() {
            "none"
        } else {
            ""
        }
    );
    for fp in &s.false_positives {
        let _ = writeln!(out, "        {}", fp);
    }
    let _ = writeln!(
        out,
        "Category:  {}/{}  ({:.1}%)   mismatches:",
        s.category_hit,
        s.category_total,
        pct(s.category_hit, s.category_total)
    );
    if s.category_mismatches.is_empty() {
        let _ = writeln!(out, "        none");
    } else {
        render_mismatches(out, &s.category_mismatches, "        ");
    }
    let _ = writeln!(
        out,
        "Exact:     {}/{}  ({:.1}%)   mismatches:",
        s.exact_hit,
        s.exact_total,
        pct(s.exact_hit, s.exact_total)
    );
    if s.exact_mismatches.is_empty() {
        let _ = writeln!(out, "        none");
    } else {
        render_mismatches(out, &s.exact_mismatches, "        ");
    }
    if s.traps_leaked.is_empty() {
        let _ = writeln!(out, "Traps:     PASS (0 leaked)");
    } else {
        let _ = writeln!(out, "Traps:     FAIL ({} leaked)", s.traps_leaked.len());
        for row in &s.traps_leaked {
            let _ = writeln!(out, "        {}", row);
        }
    }
    let _ = writeln!(out, "---------------------------------------------------");
}

fn fixture(name: &str) -> String {
    format!("{}/../../test_data/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn sample_text() -> String {
    let path = format!("{}/../../sample.txt", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_default()
}

fn write_report(body: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test_data/gold_report.md");
    let mut md = String::new();
    let _ = writeln!(md, "# Gold suite report (span-level + exact category)\n");
    let _ = writeln!(md, "Generated by `pii-core` `gold_suite` test.\n");
    let _ = writeln!(md, "```text");
    md.push_str(body);
    let _ = writeln!(md, "```");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, md);
}

#[test]
fn gold_suite_comparable_metrics_all_paths() {
    assert_eq!(
        GOLD.len(),
        54,
        "gold entity list must stay at 54 (50 PII + 4 network/crypto) for comparison"
    );

    let text = sample_text();
    let mut scores: Vec<PathScore> = Vec::new();

    scores.push(score_path("PASTE: sample.txt", &text));
    scores.push(score_path(
        "TXT: onboarding_dossier.txt",
        &extract_text(&fixture("onboarding_dossier.txt")).unwrap_or_default(),
    ));
    scores.push(score_path(
        "DOCX: onboarding_dossier.docx",
        &extract_text(&fixture("onboarding_dossier.docx")).unwrap_or_default(),
    ));
    scores.push(score_path(
        "PDF: onboarding_dossier.pdf",
        &extract_text(&fixture("onboarding_dossier.pdf")).unwrap_or_default(),
    ));

    let mut report = String::new();
    for s in &scores {
        render_path(s, &mut report);
    }

    let _ = writeln!(report, "=== SUMMARY ===");
    let _ = writeln!(
        report,
        "Exact = raw token class vs expected_exact_class (Email_1→EMAIL, Ssn_1→SSN)."
    );
    let _ = writeln!(
        report,
        "{:<8} {:<10} {:<11} {:<9} {:<8} {}",
        "Path", "Recall", "Precision", "Category", "Exact", "Traps"
    );
    for s in &scores {
        let short = s.label.split(':').next().unwrap_or("").trim().to_string();
        let traps = if s.traps_leaked.is_empty() {
            "PASS"
        } else {
            "FAIL"
        };
        let _ = writeln!(
            report,
            "{:<8} {:<10} {:<11} {:<9} {:<8} {}",
            short,
            format!("{}/{}", s.recall_hit, s.recall_total),
            format!("{}/{}", s.precision_hit, s.precision_total),
            format!("{}/{}", s.category_hit, s.category_total),
            format!("{}/{}", s.exact_hit, s.exact_total),
            traps
        );
    }

    print!("{}", report);
    write_report(&report);

    for s in &scores {
        assert!(
            s.traps_leaked.is_empty(),
            "{} trap leakage: {}",
            s.label,
            s.traps_leaked.join("; ")
        );
    }
}

/// CHANGE 5 — no two distinct mapping values may share one token (session collision).
/// Must hold for every input path, not only sample.txt.
#[test]
fn gold_suite_no_session_token_collision() {
    let mut sources: Vec<(String, String)> = vec![("sample.txt".into(), sample_text())];
    for name in [
        "onboarding_dossier.txt",
        "onboarding_dossier.docx",
        "onboarding_dossier.pdf",
    ] {
        let path = fixture(name);
        let text = extract_text(&path).unwrap_or_default();
        sources.push((name.to_string(), text));
    }

    for (label, text) in sources {
        let vault = Vault::new().expect("vault");
        let encoded = vault.encode(&text).expect("encode");

        let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for m in &encoded.mappings {
            if let Some(existing) = seen.get(&m.token) {
                if existing != &m.value {
                    panic!(
                        "{label}: session collision token {} used for both {:?} and {:?}",
                        m.token, existing, m.value
                    );
                }
            }
            seen.insert(m.token.clone(), m.value.clone());
        }

        let mut by_class: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for m in &encoded.mappings {
            if let Some(existing) = by_class.get(&m.class) {
                if existing != &m.value {
                    panic!(
                        "{label}: display class collision [{:?}] for {:?} and {:?} (token {:?})",
                        m.class, existing, m.value, m.token
                    );
                }
            }
            by_class.insert(m.class.clone(), m.value.clone());
        }

        // Primary host 10.0.14.22 must get a lower Ip_address_N than backup 10.0.14.23.
        let find_ip = |needle: &str| -> Option<(String, usize)> {
            encoded
                .mappings
                .iter()
                .find(|m| m.value.contains(needle))
                .map(|m| (m.class.clone(), m.first_offset))
        };
        if let (Some((c22, _)), Some((c23, _))) = (find_ip("10.0.14.22"), find_ip("10.0.14.23")) {
            let n22 = c22.rsplit('_').next().unwrap_or("0").parse::<u32>().unwrap_or(0);
            let n23 = c23.rsplit('_').next().unwrap_or("0").parse::<u32>().unwrap_or(0);
            assert_ne!(c22, c23, "{label}: 10.0.14.22 and 10.0.14.23 share class {c22}");
            assert!(
                n22 < n23,
                "{label}: primary 10.0.14.22 should have lower token number than backup 10.0.14.23: [{c22}] vs [{c23}]"
            );
        }
    }
}

/// BUG 1 — full name span must be captured (no trailing "atel" / "hen" / "Department").
#[test]
fn gold_suite_full_name_span_hiring_manager() {
    let vault = Vault::new().expect("vault");
    let encoded = vault.encode("Hiring manager: Priya Patel").expect("encode");
    assert_eq!(
        encoded.redacted_text, "Hiring manager: [Name_1]",
        "partial name capture leaked tail: mappings {:?}",
        encoded.mappings
    );
    assert!(
        encoded
            .mappings
            .iter()
            .any(|m| m.value.contains("Priya Patel")),
        "full name not in mappings: {:?}",
        encoded.mappings
    );
}

#[test]
fn gold_suite_full_name_span_emergency_contact() {
    let vault = Vault::new().expect("vault");
    let encoded = vault
        .encode("Emergency contact: Michael Chen (spouse)")
        .expect("encode");
    assert_eq!(
        encoded.redacted_text, "Emergency contact: [Name_1] (spouse)",
        "partial name capture: mappings {:?}",
        encoded.mappings
    );
}

#[test]
fn gold_suite_full_name_span_reference_contact() {
    let vault = Vault::new().expect("vault");
    let encoded = vault
        .encode("Reference contact: Prof. James O'Brien-Smith")
        .expect("encode");
    assert_eq!(
        encoded.redacted_text, "Reference contact: [Name_1]",
        "partial/overlong name capture: mappings {:?}",
        encoded.mappings
    );
    assert!(
        encoded
            .mappings
            .iter()
            .any(|m| m.value.contains("O'Brien-Smith")),
        "full hyphenated name missing: {:?}",
        encoded.mappings
    );
}

/// BUG 2 — print PDF vs DOCX name mappings for diagnosis (no assert on numbering).
#[test]
fn gold_suite_report_pdf_docx_name_diff() {
    let docx = extract_text(&fixture("onboarding_dossier.docx")).unwrap_or_default();
    let pdf = extract_text(&fixture("onboarding_dossier.pdf")).unwrap_or_default();
    let v1 = Vault::new().expect("vault");
    let v2 = Vault::new().expect("vault");
    let e_docx = v1.encode(&docx).expect("docx encode");
    let e_pdf = v2.encode(&pdf).expect("pdf encode");

    let names = |out: &EncodedOutput| -> Vec<(String, String, usize)> {
        out.mappings
            .iter()
            .filter(|m| m.class.contains("Name"))
            .map(|m| (m.class.clone(), m.value.clone(), m.first_offset))
            .collect()
    };
    let d = names(&e_docx);
    let p = names(&e_pdf);
    println!("DOCX names ({}):", d.len());
    for row in &d {
        println!("  {row:?}");
    }
    println!("PDF names ({}):", p.len());
    for row in &p {
        println!("  {row:?}");
    }
    let d_vals: std::collections::HashSet<_> = d.iter().map(|r| r.1.clone()).collect();
    let p_vals: std::collections::HashSet<_> = p.iter().map(|r| r.1.clone()).collect();
    println!("DOCX-only name values: {:?}", d_vals.difference(&p_vals));
    println!("PDF-only name values: {:?}", p_vals.difference(&d_vals));
    println!("--- DOCX redacted (Hiring/Emergency/Reference/sigs/Name_) ---");
    for line in e_docx.redacted_text.lines() {
        if line.contains("Hiring")
            || line.contains("Emergency")
            || line.contains("Reference contact")
            || line.contains("/s/")
            || line.contains("Name_")
        {
            println!("  {line:?}");
        }
    }
}
