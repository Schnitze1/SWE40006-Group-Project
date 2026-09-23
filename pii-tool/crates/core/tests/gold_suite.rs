//! Unified gold suite — the historical ~50-entity onboarding dossier (sample.txt)
//! plus trap items, scored on paste and every upload path so numbers are comparable.
//!
//! Run: cargo test -p pii-core --test gold_suite -- --nocapture

use pii_core::extract::extract_text;
use pii_core::vault::Vault;

struct Gold {
    id: &'static str,
    kind: &'static str,
    needle: &'static str,
    should_detect: bool,
}

/// 50 intentional PII spans from sample.txt / onboarding_dossier (the set behind
/// the old "31 of ~50 (~62% recall)" measurement).
const GOLD: &[Gold] = &[
    // Names (8)
    Gold { id: "n1", kind: "Name", needle: "Sarah Elizabeth Mitchell", should_detect: true },
    Gold { id: "n2", kind: "Name", needle: "Sarah E. Mitchell", should_detect: true },
    Gold { id: "n3", kind: "Name", needle: "Sarah Mitchell", should_detect: true },
    Gold { id: "n4", kind: "Name", needle: "Priya Patel", should_detect: true },
    Gold { id: "n5", kind: "Name", needle: "Michael Chen", should_detect: true },
    Gold { id: "n6", kind: "Name", needle: "James O'Brien-Smith", should_detect: true },
    Gold { id: "n7", kind: "Name", needle: "José Müller", should_detect: true },
    Gold { id: "n8", kind: "Name", needle: "张伟", should_detect: true },
    // Dates (5)
    Gold { id: "d1", kind: "Date", needle: "14/03/1988", should_detect: true },
    Gold { id: "d2", kind: "Date", needle: "12 Nov 2031", should_detect: true },
    Gold { id: "d3", kind: "Date", needle: "2026-09-24", should_detect: true },
    Gold { id: "d4", kind: "Date", needle: "12 Oct 2026", should_detect: true },
    Gold { id: "d5", kind: "Date", needle: "22 Sept 2026", should_detect: true },
    // Phones (6)
    Gold { id: "p1", kind: "Phone", needle: "+61 412 345 678", should_detect: true },
    Gold { id: "p2", kind: "Phone", needle: "(02) 9876 5432", should_detect: true },
    Gold { id: "p3", kind: "Phone", needle: "+1-617-555-0198", should_detect: true },
    Gold { id: "p4", kind: "Phone", needle: "617.555.0142", should_detect: true },
    Gold { id: "p5", kind: "Phone", needle: "+49 30 1234 5678", should_detect: true },
    Gold { id: "p6", kind: "Phone", needle: "+86 138 0013 8000", should_detect: true },
    // Emails (7)
    Gold { id: "e1", kind: "Email", needle: "sarah.mitchell+onboarding@example.com", should_detect: true },
    Gold { id: "e2", kind: "Email", needle: "s.mitchell@university.edu", should_detect: true },
    Gold { id: "e3", kind: "Email", needle: "priya.patel@university.edu", should_detect: true },
    Gold { id: "e4", kind: "Email", needle: "m.chen@personal-domain.io", should_detect: true },
    Gold { id: "e5", kind: "Email", needle: "j.obrien-smith@unimelb.edu.au", should_detect: true },
    Gold { id: "e6", kind: "Email", needle: "jose.muller@firma.de", should_detect: true },
    Gold { id: "e7", kind: "Email", needle: "zhang.wei@example.cn", should_detect: true },
    // Identifiers (10)
    Gold { id: "i1", kind: "Passport", needle: "PA1234567", should_detect: true },
    Gold { id: "i2", kind: "Ssn", needle: "123-45-6789", should_detect: true },
    Gold { id: "i3", kind: "Tfn", needle: "123 456 789", should_detect: true },
    Gold { id: "i4", kind: "Medicare", needle: "2123 45670 1", should_detect: true },
    Gold { id: "i5", kind: "Dln", needle: "S12345678", should_detect: true },
    Gold { id: "i6", kind: "EmployeeId", needle: "EMP-0098421", should_detect: true },
    Gold { id: "i7", kind: "Payroll", needle: "062-000", should_detect: true },
    Gold { id: "i8", kind: "Payroll", needle: "1234 5678", should_detect: true },
    Gold { id: "i9", kind: "Iban", needle: "GB29 NWBK 6016 1331 9268 19", should_detect: true },
    Gold { id: "i10", kind: "Swift", needle: "NWBKGB2L", should_detect: true },
    // Cards (3) — expiry/CVV intentionally excluded
    Gold { id: "c1", kind: "CreditCard", needle: "4111-1111-1111-1111", should_detect: true },
    Gold { id: "c2", kind: "CreditCard", needle: "3782 822463 10005", should_detect: true },
    Gold { id: "c3", kind: "CreditCard", needle: "5555 5555 5555 4444", should_detect: true },
    // Locations (8)
    Gold { id: "l1", kind: "Location", needle: "42 Beacon Street", should_detect: true },
    Gold { id: "l2", kind: "Location", needle: "Boston, MA 02108", should_detect: true },
    Gold { id: "l3", kind: "Location", needle: "Unit 7/15 Harbour Esplanade", should_detect: true },
    Gold { id: "l4", kind: "Location", needle: "Docklands VIC 3008", should_detect: true },
    Gold { id: "l5", kind: "Location", needle: "100 Cambridge Street", should_detect: true },
    Gold { id: "l6", kind: "Location", needle: "Boston MA 02114", should_detect: true },
    Gold { id: "l7", kind: "Location", needle: "Parkville VIC 3010", should_detect: true },
    Gold { id: "l8", kind: "Location", needle: "1 Parliament Square", should_detect: true },
    // Network (3) to reach 50
    Gold { id: "x1", kind: "IpAddress", needle: "10.0.14.22", should_detect: true },
    Gold { id: "x2", kind: "IpAddress", needle: "10.0.14.0/24", should_detect: true },
    Gold { id: "x3", kind: "Date", needle: "24 September 2026", should_detect: true },
    // Traps — must NOT be redacted
    Gold { id: "t1", kind: "Ref", needle: "ABC-1234", should_detect: false },
    Gold { id: "t2", kind: "Version", needle: "1.2.3", should_detect: false },
    Gold { id: "t3", kind: "Ref", needle: "INV-2026-0918", should_detect: false },
    Gold { id: "t4", kind: "Ref", needle: "PO-88-4412", should_detect: false },
    Gold { id: "t5", kind: "Ref", needle: "CASE-2026-0042", should_detect: false },
    Gold { id: "t6", kind: "Ref", needle: "JIRA-9821", should_detect: false },
    Gold { id: "t7", kind: "Org", needle: "Microsoft Corporation", should_detect: false },
    Gold { id: "t8", kind: "Phrase", needle: "salary band", should_detect: false },
    Gold { id: "t9", kind: "CardExpiry", needle: "09/28", should_detect: false },
];

struct Score {
    tp: usize,
    fn_: usize,
    fp: usize,
    tn: usize,
    misses: Vec<String>,
    false_hits: Vec<String>,
}

fn score_text(text: &str) -> Score {
    let vault = Vault::new().expect("vault");
    let encoded = vault.encode(text).expect("encode");
    let mut s = Score {
        tp: 0,
        fn_: 0,
        fp: 0,
        tn: 0,
        misses: Vec::new(),
        false_hits: Vec::new(),
    };

    for g in GOLD {
        let in_source = text.contains(g.needle);
        // Extract loss (PDF/DOCX dropped the span) is a miss, not a detection.
        if g.should_detect && !in_source {
            s.fn_ += 1;
            s.misses.push(format!("{}:{}(extract-loss)", g.id, g.kind));
            continue;
        }
        if !g.should_detect && !in_source {
            s.tn += 1;
            continue;
        }

        // Leak-based recall: if the gold string is gone from redacted output,
        // some token covered it (including larger spans / collapsed name forms).
        let leaked = encoded.redacted_text.contains(g.needle);

        if g.should_detect {
            if !leaked {
                s.tp += 1;
            } else {
                s.fn_ += 1;
                s.misses.push(format!("{}:{}", g.id, g.kind));
            }
        } else if leaked {
            s.tn += 1;
        } else {
            s.fp += 1;
            s.false_hits.push(format!("{}:{}", g.id, g.kind));
        }
    }
    s
}

fn report(label: &str, s: &Score) -> (f64, f64) {
    let precision = if s.tp + s.fp == 0 {
        1.0
    } else {
        s.tp as f64 / (s.tp + s.fp) as f64
    };
    let recall = if s.tp + s.fn_ == 0 {
        1.0
    } else {
        s.tp as f64 / (s.tp + s.fn_) as f64
    };
    let f1 = if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    };
    let pii: usize = GOLD.iter().filter(|g| g.should_detect).count();
    let traps: usize = GOLD.iter().filter(|g| !g.should_detect).count();
    println!("=== {label} ===");
    println!(
        "TP={}/{}  FN={}  FP={}/{}  TN={}",
        s.tp, pii, s.fn_, s.fp, traps, s.tn
    );
    println!(
        "precision={:.1}%  recall={:.1}%  F1={:.1}%",
        precision * 100.0,
        recall * 100.0,
        f1 * 100.0
    );
    if !s.misses.is_empty() {
        println!("missed ({}): {}", s.misses.len(), s.misses.join(", "));
    }
    if !s.false_hits.is_empty() {
        println!("false positives: {}", s.false_hits.join(", "));
    }
    println!();
    (precision, recall)
}

fn fixture(name: &str) -> String {
    format!("{}/../../test_data/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn sample_text() -> String {
    let path = format!("{}/../../sample.txt", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).expect("read sample.txt")
}

#[test]
fn gold_suite_comparable_metrics_all_paths() {
    let text = sample_text();
    assert_eq!(
        GOLD.iter().filter(|g| g.should_detect).count(),
        50,
        "gold PII list must stay at 50 for historical comparison"
    );

    let paste = score_text(&text);
    let (p_paste, r_paste) = report("PASTE sample.txt (historical 50-entity set)", &paste);

    let from_txt = extract_text(&fixture("onboarding_dossier.txt")).expect("txt");
    let txt = score_text(&from_txt);
    let (p_txt, r_txt) = report("UPLOAD onboarding_dossier.txt", &txt);

    let from_docx = extract_text(&fixture("onboarding_dossier.docx")).expect("docx");
    let docx = score_text(&from_docx);
    let (p_docx, r_docx) = report("UPLOAD onboarding_dossier.docx", &docx);

    let from_pdf = extract_text(&fixture("onboarding_dossier.pdf")).expect("pdf");
    let pdf = score_text(&from_pdf);
    let (p_pdf, r_pdf) = report("UPLOAD onboarding_dossier.pdf", &pdf);

    // Precision must stay high everywhere (trap list).
    for (label, p) in [
        ("paste", p_paste),
        ("txt", p_txt),
        ("docx", p_docx),
        ("pdf", p_pdf),
    ] {
        assert!(p >= 0.95, "{label} precision {p} dropped below 95%");
    }

    // Historical tally was "~31 of ~50" (~62%) on this same sample.txt, but that
    // count was informal (mapping hits, not strict leak-based gold). Strict gold
    // currently lands at 28/50 (56%). Guard against falling further.
    assert!(
        r_paste >= 0.55,
        "paste recall {r_paste} fell below the strict-gold baseline (0.55)"
    );
    let _ = (r_txt, r_docx, r_pdf);
}
