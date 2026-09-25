# PII Anonymisation Tool POC


## How to run

```bash
cargo run -p pii-cli
```

## Algorithm Summary (v0.1.0)

Deterministic, rules-based PII detection and reversible tokenisation.

**Verified metrics (gold suite, 54 entities + 17 traps):**
- Recall: 98.1% (53/54)
- Precision: 100% (52/52)
- Exact-category correctness: 100%
- Trap leakage: 0
- Token collisions: 0 (asserted on all four input paths)

**Known limitations (documented, not bugs):**
- No BTC detector — only ETH addresses detected
- PDF extraction loses CJK glyphs (张伟 → ❍❍) — CJK only works on TXT/DOCX
- PDF line reordering preserves token numbers but may reorder lines in output
- Scanned/image-only PDFs return NoTextLayer and are out of scope
- Card expiry/CVV intentionally not redacted
- Product codes, versions, ticket IDs intentionally not redacted

### Person-name detection (NER)

Free-text person names (e.g. `Priya Patel`, `Michael Chen`) are only partially
reliable. The `gaze-pii` NER / name recognizers miss many bare names and can
under- or over-capture spans around titles and role phrases. This is a known
limitation of the underlying detection model.

Until NER coverage improves, the Vault feature (manual token entry) is the
intended workaround: operators can register a token mapping for any name the
automatic pipeline missed so encode/decode still round-trips that value.

Do not treat “all person names in the document were redacted” as a guarantee of
this POC; verify with `View current session mapping` and add missing names via
the Vault when needed.

