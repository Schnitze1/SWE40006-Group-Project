use std::path::Path;

#[derive(Debug, PartialEq)]
pub enum ExtractError {
    FileNotFound(String),
    UnsupportedFormat(String),
    PdfError(String),
    DocxError(String),
    NoTextLayer(String),
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
            // We assume there is a way to get the number of pages, e.g. `page_count()`
            for page_index in 0..doc.page_count().map_err(|e| ExtractError::PdfError(e.to_string()))? {
                let page_text = doc.extract_text(page_index)
                    .map_err(|e| ExtractError::PdfError(e.to_string()))?;
                
                full_text.push_str(&page_text);
                full_text.push('\n');
            }

            if full_text.trim().is_empty() {
                return Err(ExtractError::NoTextLayer(
                    "No text layer found. Scanned/image-only PDFs are not supported.".to_string(),
                ));
            }

            Ok(full_text)
        }
        "docx" => {
            let doc = ooxml_wml::Document::open(path)
                .map_err(|e| ExtractError::DocxError(e.to_string()))?;
            
            let text = doc.text();
                
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
}
