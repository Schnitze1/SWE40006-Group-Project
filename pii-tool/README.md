# PII Anonymisation Tool POC


## How to run

```bash
cargo run -p pii-cli
```

## Known limitations

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
