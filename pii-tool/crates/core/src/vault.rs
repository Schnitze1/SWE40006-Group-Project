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

/// Attach a trailing `/prefix` (CIDR) to the preceding IP token so nothing dangles.
fn absorb_cidr_suffix(text: &str, mappings: &mut [TokenMapping]) -> String {
    let capacity = text.len();
    let mut out = String::with_capacity(capacity);
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            if let Some(close) = text[i..].find(']') {
                let close_abs = i + close;
                let token = &text[i..=close_abs];
                let class = &text[i + 1..close_abs];
                let end = close_abs + 1;
                // `/24` immediately after the token?
                if end < bytes.len() && bytes[end] == b'/' {
                    let mut p = end + 1;
                    while p < bytes.len() && bytes[p].is_ascii_digit() {
                        p += 1;
                    }
                    if p > end + 1 {
                        let prefix = &text[end..p]; // "/24"
                        if let Some(m) = mappings
                            .iter_mut()
                            .find(|m| format!("[{}]", m.class) == token)
                        {
                            m.value.push_str(prefix);
                        }
                        out.push_str(token);
                        i = p;
                        continue;
                    }
                }
                out.push_str(token);
                let _ = class;
                i = end;
                continue;
            }
        }
        let ch_len = text[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        out.push_str(&text[i..i + ch_len]);
        i += ch_len;
    }
    out
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

/// Org / company names — never treat "Microsoft Corporation" / "BHP Group Limited" as PII names.
fn is_org_like_name(value: &str) -> bool {
    const ORG_LAST: [&str; 10] = [
        "Corporation",
        "Corp",
        "Corp.",
        "Inc",
        "Inc.",
        "Ltd",
        "Ltd.",
        "Limited",
        "Group",
        "LLC",
    ];
    const ORG_WORDS: [&str; 8] = [
        "Corporation",
        "Corp",
        "Inc",
        "Ltd",
        "Limited",
        "Group",
        "LLC",
        "Company",
    ];
    let words: Vec<&str> = value.split_whitespace().collect();
    if words.len() < 2 {
        return false;
    }
    if let Some(last) = words.last() {
        if ORG_LAST.contains(last) {
            return true;
        }
    }
    words.iter().any(|w| {
        let bare = w.trim_end_matches(['.', ',']);
        ORG_WORDS.contains(&bare)
    })
}

/// Numeric suffix of `Name_2` / `Email_10` — lower wins when collapsing variants.
fn token_counter(class: &str) -> u32 {
    class
        .rsplit('_')
        .next()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(u32::MAX)
}

fn is_name_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '\'' || c == '-' || c == '_'
}

fn is_name_part(s: &str) -> bool {
    s.len() >= 3
        && s.chars()
            .all(|c| c.is_alphabetic() || c == '\'' || c == '-')
}

/// True when `pos` sits inside an existing `[Token]` (already redacted).
fn inside_token(text: &str, pos: usize) -> bool {
    let before = &text[..pos];
    let open = before.rfind('[');
    let close = before.rfind(']');
    match (open, close) {
        (Some(o), Some(c)) => o > c,
        (Some(_), None) => true,
        _ => false,
    }
}

/// If a name title sits immediately before `name_start`, return the title start.
fn extend_title_start(text: &str, name_start: usize) -> usize {
    const TITLES: [&str; 12] = [
        "Prof.", "Prof", "Dr.", "Dr", "Mr.", "Mr", "Mrs.", "Mrs", "Ms.", "Ms", "Mx.", "Mx",
    ];
    let before = &text[..name_start];
    let trimmed = before.trim_end();
    for title in TITLES {
        if let Some(rest) = trimmed.strip_suffix(title) {
            let boundary_ok = rest
                .chars()
                .next_back()
                .map(|c| !is_name_word_char(c))
                .unwrap_or(true);
            if boundary_ok {
                return rest.len();
            }
        }
    }
    name_start
}

/// Replace whole-word `needle` with `token`. Optionally include a preceding title.
fn replace_whole_words(text: &str, needle: &str, token: &str, allow_title: bool) -> String {
    if needle.is_empty() {
        return text.to_string();
    }
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut search = 0;
    while search < text.len() {
        let Some(rel) = text[search..].find(needle) else {
            break;
        };
        let start = search + rel;
        let end = start + needle.len();
        let before_ok = start == 0
            || !text[..start]
                .chars()
                .next_back()
                .is_some_and(is_name_word_char);
        let after_ok = end >= text.len()
            || !text[end..].chars().next().is_some_and(is_name_word_char);
        if before_ok && after_ok && !inside_token(text, start) {
            let mut repl_start = start;
            if allow_title {
                repl_start = extend_title_start(text, start);
            }
            spans.push((repl_start, end));
        }
        search = end;
    }
    let mut out = text.to_string();
    for (start, end) in spans.into_iter().rev() {
        out.replace_range(start..end, token);
    }
    out
}

/// Redact bare first/last names that match an already-tokenised PERSON mapping.
/// Case-sensitive whole words only; never touches text inside `[...]`.
fn expand_known_person_names(text: &str, mappings: &[TokenMapping]) -> String {
    let mut out = text.to_string();
    for m in mappings {
        let is_name = m.class.contains("Name") || m.class.contains("name");
        if !is_name {
            continue;
        }
        let token = format!("[{}]", m.class);
        let core = person_core_name(&m.value);
        let mut parts = core.split_whitespace();
        let raw_first = parts.next().unwrap_or("");
        let first = raw_first.trim_matches(|c: char| !is_name_word_char(c));
        if is_name_part(first) {
            out = replace_whole_words(&out, first, &token, false);
        }
        let mut last = "";
        for part in core.split_whitespace() {
            last = part;
        }
        let last = last.trim_matches(|c: char| !is_name_word_char(c));
        if is_name_part(last) && last != first {
            out = replace_whole_words(&out, last, &token, true);
        }
    }
    out
}

/// Class family prefix before the `_<N>` counter (e.g. `Name_3` → `Name`).
fn class_family(class: &str) -> &str {
    match class.rfind('_') {
        Some(i) => &class[..i],
        None => class,
    }
}

/// Stable sort key so IP hosts number by address (10.0.14.22 before 10.0.14.23)
/// even when PDF extract reorders lines. Other families use first_offset.
fn renumber_sort_key(m: &TokenMapping) -> (u8, String, String, usize) {
    let family = class_family(&m.class);
    let family_l = family.to_lowercase();
    if family_l.contains("ip") {
        // Numeric IPv4 first (last octets), then CIDR, then IPv6 / other.
        let v = m.value.trim();
        if let Some(key) = ipv4_sort_key(v) {
            return (0, key, String::new(), m.first_offset);
        }
        return (1, String::new(), v.to_string(), m.first_offset);
    }
    let _ = family;
    (2, String::new(), String::new(), m.first_offset)
}

fn ipv4_sort_key(value: &str) -> Option<String> {
    let host = value.split('/').next().unwrap_or("");
    let mut parts = host.split('.');
    let a = parts.next()?.parse::<u32>().ok()?;
    let b = parts.next()?.parse::<u32>().ok()?;
    let c = parts.next()?.parse::<u32>().ok()?;
    let d = parts.next()?.parse::<u32>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(format!("{a:03}.{b:03}.{c:03}.{d:03}"))
}

/// Reassign `Class_N` so N is unique per family and stable (offset / IP value).
/// Uses a two-phase rename via unique placeholders so swaps cannot clobber.
fn renumber_tokens_by_first_offset(mappings: &mut [TokenMapping], text: &mut String) {
    let mut order: Vec<usize> = (0..mappings.len()).collect();
    order.sort_by(|&a, &b| {
        let ka = renumber_sort_key(&mappings[a]);
        let kb = renumber_sort_key(&mappings[b]);
        ka.cmp(&kb)
    });

    let mut counters: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut renames: Vec<(String, String)> = Vec::new();
    let mut updates: Vec<(usize, String)> = Vec::new();

    for idx in order {
        let m = &mappings[idx];
        let family = class_family(&m.class).to_string();
        let family_key = family.clone();
        let entry = counters.entry(family_key).or_insert(0);
        *entry += 1;
        let n = *entry;
        let new_class = format!("{family}_{n}");
        if new_class != m.class {
            let old = m.class.clone();
            renames.push((old, new_class.clone()));
        }
        updates.push((idx, new_class));
    }

    // Phase 1: old → unique placeholder (longest old first so Name_10 ≠ Name_1).
    renames.sort_by(|a, b| {
        let left = b.0.len();
        let right = a.0.len();
        left.cmp(&right)
    });
    let mut staged: Vec<(String, String)> = Vec::new(); // (placeholder, new)
    for (i, (old, new)) in renames.iter().enumerate() {
        let placeholder = format!("\u{E000}TOK{i}\u{E001}");
        let from = format!("[{old}]");
        *text = text.replace(&from, &placeholder);
        let ph = placeholder.clone();
        staged.push((ph, new.clone()));
    }
    // Phase 2: placeholder → new.
    for (placeholder, new) in &staged {
        let to = format!("[{new}]");
        *text = text.replace(placeholder, &to);
    }
    for (idx, new_class) in updates {
        mappings[idx].class = new_class;
    }
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
        const EXTRA_RULES: &str = include_str!("../rules/poc_extra.toml");

        // Write embedded rules to a runtime path (works on Lambda's /tmp and
        // locally via the OS temp dir). Written once per process.
        let rules_path = std::env::temp_dir().join("poco_poc_extra.toml");
        if !rules_path.exists() {
            std::fs::write(&rules_path, EXTRA_RULES).map_err(|e| {
                VaultError::Pipeline(format!("failed to write embedded rules: {}", e))
            })?;
        }

        let pipeline = CorePipelineConfig::new()
            .with_locale(&[LocaleTag::EnUs])
            .with_bundled_rulepack("locale-en")
            .with_rulepack_path(&rules_path)
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
            let value_key = m.value.clone();
            let entry = value_to_class.entry(value_key).or_insert_with(|| m.class.clone());
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

        // Collapse titled / bare / case variants of one person onto a single token
        // (Mask shared-label behaviour: "Dr. Priya Nair" and "Priya Nair" are one entity).
        let mut core_to_class: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for m in &deduped {
            let is_name = m.class.contains("Name") || m.class.contains("name");
            if !is_name {
                continue;
            }
            let core = person_core_name(&m.value).to_lowercase();
            if core.is_empty() {
                continue;
            }
            match core_to_class.get(&core) {
                None => {
                    let class_name = m.class.clone();
                    core_to_class.insert(core, class_name);
                }
                Some(existing) => {
                    // Prefer the earliest session counter (Name_1 beats Name_2) so
                    // cross-document variants keep the first-seen token.
                    let incoming = token_counter(&m.class);
                    let current = token_counter(existing);
                    if incoming < current {
                        let class_name = m.class.clone();
                        core_to_class.insert(core, class_name);
                    }
                }
            }
        }
        for m in &deduped {
            let is_name = m.class.contains("Name") || m.class.contains("name");
            if !is_name {
                continue;
            }
            let core = person_core_name(&m.value).to_lowercase();
            let Some(canonical) = core_to_class.get(&core) else {
                continue;
            };
            if *canonical == m.class {
                continue;
            }
            let from = format!("[{}]", m.class);
            let to = format!("[{canonical}]");
            readable_redacted_text = readable_redacted_text.replace(&from, &to);
        }
        let mut collapsed: Vec<TokenMapping> = Vec::new();
        for m in deduped {
            let is_name = m.class.contains("Name") || m.class.contains("name");
            let canonical = if is_name {
                let core = person_core_name(&m.value).to_lowercase();
                core_to_class
                    .get(&core)
                    .cloned()
                    .unwrap_or_else(|| m.class.clone())
            } else {
                m.class.clone()
            };
            if let Some(existing) = collapsed.iter_mut().find(|d| {
                d.class == canonical
                    && person_core_name(&d.value).to_lowercase()
                        == person_core_name(&m.value).to_lowercase()
            }) {
                if m.first_offset < existing.first_offset {
                    existing.first_offset = m.first_offset;
                }
            } else {
                collapsed.push(TokenMapping {
                    token: m.token,
                    value: m.value,
                    class: canonical,
                    first_offset: m.first_offset,
                });
            }
        }
        // Reject org/company false positives ("Microsoft Corporation").
        let mut kept: Vec<TokenMapping> = Vec::new();
        for m in collapsed {
            let is_name = m.class.contains("Name") || m.class.contains("name") || m.class.contains("Location");
            if is_org_like_name(&m.value) {
                let token = format!("[{}]", m.class);
                readable_redacted_text = readable_redacted_text.replace(&token, &m.value);
                continue;
            }
            let _ = is_name;
            kept.push(m);
        }
        let mut deduped = kept;

        // Strip any surrounding < > left after replacing wrapped tokens (e.g. `<[Email_1]>`).
        readable_redacted_text = readable_redacted_text
            .replace("<<[", "[")
            .replace("]>>", "]")
            .replace("<[", "[")
            .replace("]>", "]");

        // CIDR: pull a dangling `/prefix` into the IP mapping so 10.0.14.0/24 is one token.
        // (core ip.v4 may win the span and leave `/24` behind.)
        readable_redacted_text = absorb_cidr_suffix(&readable_redacted_text, &mut deduped);

        // CHANGE 2: display order follows first appearance in the source text.
        deduped.sort_by_key(|m| m.first_offset);

        // Session-known-name lookup: bare first/last names that match an existing
        // PERSON mapping (e.g. "Priya" after "Priya Patel") reuse that token.
        readable_redacted_text = expand_known_person_names(&readable_redacted_text, &deduped);

        // Keep Class_N counters aligned with first_offset inside each family
        // (Name_1 is always the earliest name in the source, …).
        renumber_tokens_by_first_offset(&mut deduped, &mut readable_redacted_text);

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
                                let token_text = readable_token.to_string();
                                replacements.push((token_text, original_value));
                            }
                        } else {
                            let token_text = readable_token.to_string();
                            if !hallucinations.contains(&token_text) {
                                hallucinations.push(token_text);
                            }
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
        replacements.sort_by(|a, b| {
            let left = b.0.len();
            let right = a.0.len();
            left.cmp(&right)
        });
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

    // ---------- Split identity / financial class names ----------

    #[test]
    fn readable_class_split_identity_and_financial() {
        assert_eq!(readable_class("custom:passport"), "Passport");
        assert_eq!(readable_class("custom:tfn"), "Tfn");
        assert_eq!(readable_class("custom:medicare"), "Medicare");
        assert_eq!(readable_class("custom:dln"), "Dln");
        assert_eq!(readable_class("custom:employee"), "Employee");
        assert_eq!(readable_class("custom:ssn"), "Ssn");
        assert_eq!(readable_class("custom:swift"), "Swift");
        assert_eq!(readable_class("custom:payroll"), "Payroll");
        assert_eq!(readable_class("custom:iban"), "Iban");
    }

    // ---------- CHANGE 2 — mapping display order by first appearance ----------

    // ---------- Session-known-name lookup (bare first / last) ----------

    #[test]
    fn session_name_expands_bare_first_name() {
        let vault = Vault::new().unwrap();
        let encoded = vault
            .encode("Hiring manager: Priya Patel. Priya will review.")
            .unwrap();
        assert_eq!(
            encoded.redacted_text,
            "Hiring manager: [Name_1]. [Name_1] will review.",
            "mappings {:?}",
            encoded.mappings
        );
    }

    #[test]
    fn session_name_expands_bare_first_name_emergency() {
        let vault = Vault::new().unwrap();
        let encoded = vault
            .encode("Emergency contact: Michael Chen. Michael is the spouse.")
            .unwrap();
        assert_eq!(
            encoded.redacted_text,
            "Emergency contact: [Name_1]. [Name_1] is the spouse.",
            "mappings {:?}",
            encoded.mappings
        );
    }

    #[test]
    fn session_name_expands_title_plus_surname() {
        let vault = Vault::new().unwrap();
        let encoded = vault
            .encode("Contact Dr. Sarah Mitchell. Dr. Mitchell will confirm.")
            .unwrap();
        assert_eq!(
            encoded.redacted_text,
            "Contact [Name_1]. [Name_1] will confirm.",
            "mappings {:?}",
            encoded.mappings
        );
    }

    #[test]
    fn session_name_whole_word_no_substring() {
        let vault = Vault::new().unwrap();
        let encoded = vault
            .encode("Contact Dr. Sarah Mitchell. The Mitchella plant grows here.")
            .unwrap();
        assert_eq!(
            encoded.redacted_text,
            "Contact [Name_1]. The Mitchella plant grows here.",
            "mappings {:?}",
            encoded.mappings
        );
    }

    #[test]
    fn session_name_skips_short_first_names() {
        let vault = Vault::new().unwrap();
        // "Ed Li" — first name too short to expand; must not wipe "Ed" elsewhere.
        let encoded = vault
            .encode("Hiring manager: Dr. Edward Li. Ed will review.")
            .unwrap();
        assert!(
            encoded.redacted_text.contains("Ed will review"),
            "short/ambiguous given name must not be expanded: {}",
            encoded.redacted_text
        );
    }

    // ---------- token order follows first_offset ----------

    #[test]
    fn mappings_sorted_by_first_offset() {
        let path = format!("{}/../../sample.txt", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let vault = Vault::new().unwrap();
        let encoded = vault.encode(&text).unwrap();
        let mut prev = 0usize;
        for m in &encoded.mappings {
            assert!(
                m.first_offset >= prev,
                "mappings not sorted by first_offset: {:?} after offset {}",
                m,
                prev
            );
            prev = m.first_offset;
        }
    }

    #[test]
    fn name_token_numbers_follow_first_offset() {
        let path = format!("{}/../../sample.txt", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let vault = Vault::new().unwrap();
        let encoded = vault.encode(&text).unwrap();
        let names: Vec<&TokenMapping> = encoded
            .mappings
            .iter()
            .filter(|m| m.class.contains("Name"))
            .collect();
        assert!(names.len() >= 2, "need multiple names: {:?}", names);
        for pair in names.windows(2) {
            let a = pair[0];
            let b = pair[1];
            let na = token_counter(&a.class);
            let nb = token_counter(&b.class);
            assert!(
                na < nb,
                "Name token numbers must increase with first_offset: {:?} ({} @ {}) then {:?} ({} @ {})",
                a.value,
                a.class,
                a.first_offset,
                b.value,
                b.class,
                b.first_offset
            );
        }
    }

    #[test]
    fn section5_person_token_before_section6_occupation() {
        // 张伟 (handwritten note) appears before "Sarah Mitchell — Senior …".
        let vault = Vault::new().unwrap();
        let encoded = vault
            .encode(
                "Please CC 张伟 (zhang.wei@example.cn).\n\
                 Occupations:\n  - Sarah Mitchell — Senior Software Engineer\n",
            )
            .unwrap();
        let zhang = encoded
            .mappings
            .iter()
            .find(|m| m.value.contains("张伟"))
            .expect("张伟 mapping");
        let sarah = encoded
            .mappings
            .iter()
            .find(|m| m.value.contains("Sarah Mitchell"))
            .expect("Sarah mapping");
        let nz = token_counter(&zhang.class);
        let ns = token_counter(&sarah.class);
        assert!(
            zhang.first_offset < sarah.first_offset,
            "fixture order: {:?}",
            encoded.mappings
        );
        assert!(
            nz < ns,
            "section-5 person must get a lower Name_N than section-6 occupation: {} vs {}",
            zhang.class,
            sarah.class
        );
    }

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

    /// DD Mon YYYY / DD Month YYYY (passport, contract, travel dates).
    #[test]
    fn date_mon_year_formats_are_detected() {
        let vault = Vault::new().unwrap();
        for date in [
            "12 Nov 2031",
            "22 Sept 2026",
            "12 Oct 2026",
            "24 September 2026",
            "1 Jan 2020",
        ] {
            let encoded = vault.encode(format!("Expires {date}.").as_str()).unwrap();
            assert!(
                encoded.mappings.iter().any(|m| m.value.contains(date)
                    && m.class.to_lowercase().contains("date")),
                "date {date:?} not detected: {:?}",
                encoded.mappings
            );
            assert!(
                !encoded.redacted_text.contains(date),
                "date {date:?} leaked: {}",
                encoded.redacted_text
            );
        }
    }

    /// IPv4 CIDR must be one token — no dangling `/24`.
    #[test]
    fn cidr_ipv4_is_one_token_no_dangling_prefix() {
        let vault = Vault::new().unwrap();
        let encoded = vault.encode("Subnet: 10.0.14.0/24").unwrap();

        assert!(
            encoded
                .mappings
                .iter()
                .any(|m| m.value.contains("10.0.14.0/24") || m.value.contains("10.0.14.0")),
            "cidr not detected: {:?}",
            encoded.mappings
        );
        assert!(
            !encoded.redacted_text.contains("/24"),
            "CIDR prefix left hanging: {}",
            encoded.redacted_text
        );
        assert!(
            !encoded.redacted_text.contains("10.0.14.0"),
            "IP leaked: {}",
            encoded.redacted_text
        );
    }

    /// Card expiry/CVV stay put (not PII without the PAN).
    #[test]
    fn card_expiry_and_cvv_left_alone() {
        let vault = Vault::new().unwrap();
        let encoded = vault
            .encode("Visa: 4111-1111-1111-1111 expiry 09/28 CVV 123")
            .unwrap();
        assert_preserved(&encoded, "09/28", "card expiry");
        assert_preserved(&encoded, "123", "cvv");
        assert_detected(&encoded, "4111-1111-1111-1111", "pan");
    }
}