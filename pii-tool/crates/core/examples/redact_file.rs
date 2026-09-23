//! Encode a text file and write ./output/redacted.txt (POC helper for demos).
use pii_core::vault::Vault;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "sample.txt".to_string());
    let text = std::fs::read_to_string(&path).expect("read input");
    let vault = Vault::new().expect("vault");
    let encoded = vault.encode(&text).expect("encode");

    println!("Redacted {} entities:", encoded.mappings.len());
    for m in &encoded.mappings {
        println!("[{}] → {}", m.class, m.value);
    }

    std::fs::create_dir_all("./output").ok();
    std::fs::write("./output/redacted.txt", &encoded.redacted_text).expect("write output");
    println!("\nWrote ./output/redacted.txt");
    println!("---\n{}", encoded.redacted_text);
}
