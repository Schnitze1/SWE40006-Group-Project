use std::path::Path;

#[derive(Debug, PartialEq)]
pub enum ExtractError {
    FileNotFound(String),
    UnsupportedFormat(String),
    PdfError(String),
    DocxError(String),
    NoTextLayer(String),
}

/// Rejoin identifiers that PDF extraction splits across newlines
/// (emails, phone digit runs). Ordinary sentence breaks are preserved.
pub fn unwrap_split_identifiers(text: &str) -> String {
    let capacity = text.len();
    let mut out = String::with_capacity(capacity);
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        out.push_str(line);
        if let Some(next) = lines.peek() {
            let prev = line.trim_end();
            let nxt = next.trim_start();
            let email_join = prev.ends_with('@')
                || (prev.ends_with('.')
                    && nxt
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_alphanumeric()));
            let digit_join = prev
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_digit())
                && nxt.chars().next().is_some_and(|c| c.is_ascii_digit());
            if !(email_join || digit_join) {
                out.push('\n');
            }
        }
    }
    if text.ends_with('\n') && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

pub fn extract_text(file_path: &str) -> Result<String, ExtractError> {
    let path = Path::new(file_path);

    if !path.exists() {
        return Err(ExtractError::FileNotFound(file_path.to_string()));
    }

    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
        .unwrap_or_default();

    match extension.as_str() {
        "pdf" => {
            let doc = pdf_oxide::PdfDocument::open(path)
                .map_err(|e| ExtractError::PdfError(e.to_string()))?;

            let mut full_text = String::new();
            for page_index in 0..doc
                .page_count()
                .map_err(|e| ExtractError::PdfError(e.to_string()))?
            {
                let page_text = doc
                    .extract_text(page_index)
                    .map_err(|e| ExtractError::PdfError(e.to_string()))?;

                full_text.push_str(&page_text);
                full_text.push('\n');
            }

            if full_text.trim().is_empty() {
                return Err(ExtractError::NoTextLayer(
                    "No text layer found. Scanned/image-only PDFs are not supported.".to_string(),
                ));
            }

            Ok(unwrap_split_identifiers(&full_text))
        }
        "docx" => {
            let doc = ooxml_wml::Document::open(path)
                .map_err(|e| ExtractError::DocxError(e.to_string()))?;

            let text = doc.text();

            Ok(text)
        }
        "txt" | "text" | "md" => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| ExtractError::DocxError(format!("read failed: {e}")))?;
            Ok(text)
        }
        _ => Err(ExtractError::UnsupportedFormat(extension)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nonexistent_file() {
        let result = extract_text("nonexistent.pdf");
        assert_eq!(
            result,
            Err(ExtractError::FileNotFound("nonexistent.pdf".to_string()))
        );
    }

    #[test]
    fn test_unwrap_rejoins_email_across_newline() {
        let raw = "Please contact sarah.mitchell@\nexample.com for details.";
        let fixed = unwrap_split_identifiers(raw);
        assert_eq!(fixed, "Please contact sarah.mitchell@example.com for details.");
    }

    #[test]
    fn test_unwrap_rejoins_phone_digits_across_newline() {
        let raw = "Her phone is +61 412\n345 678.";
        let fixed = unwrap_split_identifiers(raw);
        assert_eq!(fixed, "Her phone is +61 412345 678.");
    }

    #[test]
    fn test_unwrap_preserves_normal_newlines() {
        let raw = "Please contact sarah.mitchell@\nexample.com for details.\n\nHer phone is +61 412\n345 678.\n\nSigned by Priya Patel on 24 September 2026.\n";
        let fixed = unwrap_split_identifiers(raw);
        assert!(fixed.contains("sarah.mitchell@example.com"));
        assert!(fixed.contains("Priya Patel on 24 September 2026.\n"));
        assert!(fixed.contains("for details.\n\nHer phone"));
    }

    fn fixture(name: &str) -> String {
        format!(
            "{}/../../test_data/{name}",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    #[test]
    fn test_txt_supported() {
        let text = extract_text(&fixture("minimal_clean.txt")).expect("txt should extract");
        assert!(text.contains("alice@example.com"));
        assert!(text.contains("+1-555-0100"));
    }

    #[test]
    fn test_txt_no_pii() {
        let text = extract_text(&fixture("no_pii.txt")).expect("txt should extract");
        assert!(text.contains("deployment pipeline"));
        assert!(!text.contains('@'));
    }

    #[test]
    fn test_linebreak_traps_email_visible_after_unwrap() {
        let text = extract_text(&fixture("linebreak_traps.pdf")).expect("pdf should extract");
        assert!(text.contains("sarah.mitchell@example.com"));
        assert!(text.contains("+61 412345 678"));
    }

    #[test]
    fn test_scanned_pdf_reports_no_text_layer() {
        let err = extract_text(&fixture("scanned_page.pdf")).unwrap_err();
        assert!(matches!(err, ExtractError::NoTextLayer(_)));
    }

    #[test]
    fn test_onboarding_docx_and_txt_agree() {
        let from_txt = extract_text(&fixture("onboarding_dossier.txt")).expect("txt");
        let from_docx = extract_text(&fixture("onboarding_dossier.docx")).expect("docx");
        assert_eq!(from_txt, from_docx);
        assert!(from_txt.contains("sarah.mitchell+onboarding@example.com"));
    }

    #[test]
    fn test_table_heavy_docx_has_rows() {
        let text = extract_text(&fixture("table_heavy.docx")).expect("docx");
        let rows = text.lines().filter(|l| l.contains('\t')).count();
        assert!(rows >= 5, "expected tab-separated rows, got {rows}");
    }

    #[test]
    fn test_multi_page_pdf_contains_all_pages() {
        let text = extract_text(&fixture("multi_page_contract.pdf")).expect("pdf");
        for n in 1..=5 {
            assert!(
                text.contains(&format!("Page {n} of 5")),
                "missing page marker {n}"
            );
        }
    }

    #[test]
    fn test_unicode_pdf_keeps_latin_and_cyrillic() {
        let text = extract_text(&fixture("unicode_mix.pdf")).expect("pdf");
        assert!(text.contains("José") || text.contains("Jose"));
        assert!(text.contains("Иван"));
        assert!(text.contains("zhang.wei@example.cn"));
    }
}
