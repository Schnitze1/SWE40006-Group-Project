use std::io::Write;
use pii_core::vault::Vault;
use pii_core::vault::TokenMapping;

#[derive(Debug, PartialEq)]
pub enum MenuAction {
    Encode,
    Decode,
    ViewMapping,
    ClearSession,
    Exit,
    Invalid,
}

pub fn parse_menu_selection(input: &str) -> MenuAction {
    match input.trim() {
        "1" => MenuAction::Encode,
        "2" => MenuAction::Decode,
        "3" => MenuAction::ViewMapping,
        "4" => MenuAction::ClearSession,
        "5" => MenuAction::Exit,
        _ => MenuAction::Invalid,
    }
}

fn read_input() -> Option<String> {
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).unwrap_or(0) == 0 {
        return None;
    }
    Some(input.trim().to_string())
}

fn read_multiline() -> Option<String> {
    let mut lines = Vec::new();
    loop {
        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).unwrap_or(0) == 0 {
            return None;
        }
        if input.trim() == ";;" {
            break;
        }
        lines.push(input);
    }
    Some(lines.join(""))
}

fn get_text_from_submenu() -> Option<String> {
    loop {
        println!("\n  1. Load from file path (.pdf, .docx, .txt)");
        println!("  2. Type/paste text (end with ;; on its own line)");
        println!("  3. Back");
        print!("Select an option: ");
        std::io::stdout().flush().ok();
        
        let choice = read_input()?;
        match choice.as_str() {
            "1" => {
                print!("Enter file path: ");
                std::io::stdout().flush().ok();
                let path = read_input()?;
                
                let p = std::path::Path::new(&path);
                if !p.exists() {
                    println!("⚠ File not found: {}", path);
                    continue;
                }

                println!("Extracting text...");
                if path.to_lowercase().ends_with(".txt") {
                    match std::fs::read_to_string(&path) {
                        Ok(text) => return Some(text),
                        Err(e) => {
                            eprintln!("Error reading txt: {}", e);
                            return None;
                        }
                    }
                } else {
                    match pii_core::extract::extract_text(&path) {
                        Ok(text) => return Some(text),
                        Err(e) => {
                            eprintln!("Error extracting text: {:?}", e);
                            return None;
                        }
                    }
                }
            }
            "2" => {
                println!("Enter text (end with ;; on its own line):");
                return read_multiline();
            }
            "3" => {
                return None;
            }
            _ => {
                println!("Invalid input.");
            }
        }
    }
}

fn main() {
    let mut vault = match Vault::new() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Failed to initialize Vault: {:?}", e);
            return;
        }
    };
    
    let mut session_mappings: Vec<TokenMapping> = Vec::new();
    
    loop {
        println!("\n=== PII Anonymiser — Proof of Concept ===");
        println!("  1. Encode a document or text");
        println!("  2. Decode an LLM response");
        println!("  3. View current session mapping");
        println!("  4. Clear session");
        println!("  5. Exit");
        print!("\nSelect an option: ");
        std::io::stdout().flush().ok();
        
        let choice_str = match read_input() {
            Some(c) => c,
            None => {
                println!("\nEOF detected. Exiting...");
                break;
            }
        };
        
        let action = parse_menu_selection(&choice_str);
        
        match action {
            MenuAction::Encode => {
                if let Some(text) = get_text_from_submenu() {
                    if text.trim().is_empty() {
                        println!("⚠ No text provided.");
                        continue;
                    }
                    
                    println!("Detecting PII...");
                    match vault.encode(&text) {
                        Ok(encoded) => {
                            let num_entities = encoded.mappings.len();
                            if num_entities == 0 {
                                println!("\n✓ No PII detected in the provided text.");
                            } else {
                                println!("\n✓ Redacted {} entities.\n", num_entities);
                                println!("Mapping:");
                                // EncodedOutput.mappings are already in first-appearance order.
                                for m in &encoded.mappings {
                                    println!("[{}] → {}", m.class, m.value);

                                    if let Some(existing) = session_mappings
                                        .iter_mut()
                                        .find(|existing| existing.token == m.token)
                                    {
                                        if m.first_offset < existing.first_offset {
                                            existing.first_offset = m.first_offset;
                                        }
                                    } else {
                                        let copy = m.clone();
                                        session_mappings.push(copy);
                                    }
                                }
                            }
                            
                            println!("\nPreview (first 20 lines of redacted text):");
                            for line in encoded.redacted_text.lines().take(20) {
                                println!("{}", line);
                            }
                            
                            if let Err(e) = std::fs::create_dir_all("./output") {
                                eprintln!("\nFailed to create output dir: {}", e);
                            } else {
                                if let Err(e) = std::fs::write("./output/redacted.txt", &encoded.redacted_text) {
                                    eprintln!("\nFailed to write output file: {}", e);
                                } else {
                                    println!("\nFull redacted text written to: ./output/redacted.txt");
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Error encoding text: {:?}", e);
                        }
                    }
                }
            }
            MenuAction::Decode => {
                if let Some(text) = get_text_from_submenu() {
                    if text.trim().is_empty() {
                        println!("⚠ No text provided.");
                        continue;
                    }
                    
                    match vault.decode(&text, &session_mappings) {
                        Ok(decoded) => {
                            println!("\n✓ Restored text:\n\n{}", decoded.restored_text);
                            if !decoded.hallucinations.is_empty() {
                                println!("\n\x1b[31m⚠ Hallucinated tokens detected (not in session mapping):\x1b[0m");
                                for h in &decoded.hallucinations {
                                    println!("\x1b[31m{} — undefined\x1b[0m", h);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Error decoding text: {:?}", e);
                        }
                    }
                }
            }
            MenuAction::ViewMapping => {
                println!();
                if session_mappings.is_empty() {
                    println!("No mappings in current session.");
                } else {
                    // CHANGE 2: first-appearance order in the source, not alpha/ID.
                    let mut ordered: Vec<&TokenMapping> = session_mappings.iter().collect();
                    ordered.sort_by_key(|m| m.first_offset);
                    println!("{:<15} {:<30} {:<15}", "Token", "Value", "Class");
                    for m in ordered {
                        let token = format!("[{}]", m.class);
                        let class_base = m.class.split('_').next().unwrap_or(&m.class);
                        println!("{:<15} {:<30} {:<15}", token, m.value, class_base);
                    }
                }
            }
            MenuAction::ClearSession => {
                match Vault::new() {
                    Ok(v) => {
                        vault = v;
                        session_mappings.clear();
                        println!("\nSession cleared.");
                    }
                    Err(e) => {
                        eprintln!("\nFailed to create new Vault: {:?}", e);
                    }
                }
            }
            MenuAction::Exit => {
                break;
            }
            MenuAction::Invalid => {
                println!("\nInvalid input.");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_menu_selection() {
        assert_eq!(parse_menu_selection("1\n"), MenuAction::Encode);
        assert_eq!(parse_menu_selection("  1  "), MenuAction::Encode);
        assert_eq!(parse_menu_selection("2\n"), MenuAction::Decode);
        assert_eq!(parse_menu_selection("3"), MenuAction::ViewMapping);
        assert_eq!(parse_menu_selection("4"), MenuAction::ClearSession);
        assert_eq!(parse_menu_selection("5"), MenuAction::Exit);
        assert_eq!(parse_menu_selection("invalid"), MenuAction::Invalid);
        assert_eq!(parse_menu_selection(""), MenuAction::Invalid);
    }
}
