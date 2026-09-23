fn main() {
    let mut vault = pii_core::vault::Vault::new().unwrap();
    
    let txt = pii_core::extract::extract_text("test_data/onboarding_dossier.txt").unwrap();
    let docx = pii_core::extract::extract_text("test_data/onboarding_dossier.docx").unwrap();
    let pdf = pii_core::extract::extract_text("test_data/onboarding_dossier.pdf").unwrap();
    
    let enc_txt = vault.encode(&txt).unwrap();
    vault = pii_core::vault::Vault::new().unwrap();
    let enc_docx = vault.encode(&docx).unwrap();
    vault = pii_core::vault::Vault::new().unwrap();
    let enc_pdf = vault.encode(&pdf).unwrap();
    
    let mut t_vals: Vec<_> = enc_txt.mappings.iter().map(|m| &m.value).collect();
    let mut d_vals: Vec<_> = enc_docx.mappings.iter().map(|m| &m.value).collect();
    let mut p_vals: Vec<_> = enc_pdf.mappings.iter().map(|m| &m.value).collect();
    
    t_vals.sort(); d_vals.sort(); p_vals.sort();
    
    println!("TXT values ({}):", t_vals.len());
    for v in &t_vals { println!("  {}", v); }
    println!("---");
    println!("DOCX values ({}):", d_vals.len());
    for v in &d_vals { if !t_vals.contains(v) { println!("  + {}", v); } }
    for v in &t_vals { if !d_vals.contains(v) { println!("  - {}", v); } }
    println!("---");
    println!("PDF values ({}):", p_vals.len());
    for v in &p_vals { if !t_vals.contains(v) { println!("  + {}", v); } }
    for v in &t_vals { if !p_vals.contains(v) { println!("  - {}", v); } }
}
