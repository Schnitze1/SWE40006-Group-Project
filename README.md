# SWE40006 — PII Anonymisation Tool

Reversible PII detection and tokenisation for safe use of documents with LLMs.

Detect PII in text (and PDF/DOCX uploads), replace it with readable tokens such as `[Email_1]` and `[Name_1]`, send the redacted text to an LLM, then restore the original values. Built as a university DevOps project (SWE40006) with a Rust core, CLI, and AWS Lambda HTTP API.

---

## Repository layout

| Path | What it is |
|---|---|
| `pii-tool/` | Cargo workspace (core engine, CLI, Lambda handler) |
| `pii-tool/crates/core` | `pii-core` — extract, detect, encode/decode (gold-tested) |
| `pii-tool/crates/cli` | Interactive CLI (`cargo run -p pii-cli`) |
| `pii-tool/crates/lambda` | `poco-lambda` — AWS Lambda HTTP API |
| `pii-tool/test_data/` | PDF/DOCX/TXT fixtures for the gold suite |
| `docs/API.md` | HTTP API contract for the website client |
| `docs/ENVIRONMENTS.md` | dev / test / staging / prod deploy map |
| `.github/workflows/deploy.yml` | CI: tests → AL2023 Lambda build → deploy → smoke |

---

## Quick start (local)

```bash
cd pii-tool
cargo test --workspace          # full suite including gold metrics
cargo run -p pii-cli            # interactive encode/decode
```

**Algorithm metrics** (54-entity gold suite + 17 traps): recall 53/54 (98.1%), precision 100%, exact-category 100%, trap leakage 0. Details: `pii-tool/README.md`.

---

## Architecture (short)

```text
file / paste text
    → extract (.pdf / .docx / .txt)
    → Vault::encode  (gaze-pii + rules)
    → redactedText + mappings[]
    → LLM (out of band)
    → Vault::decode  (session / mappings)
    → restored text
```

- Detection is **rules-based** (gaze-pii + `poc_extra.toml`), not a neural NER.
- Tokens are reversible; mappings must never be written to disk in the Lambda.
- HTTP API is **paste-text only** in v1 — extract files on the client or CLI first.

---

## Deployment

| Trigger | Environment | Lambda | API stage |
|---|---|---|---|
| push `develop` | dev | `poco-pii-api-handler-dev` | `/dev` |
| push `test/**` | test | `poco-pii-api-handler-test` | `/test` |
| push `release/**` | staging | `poco-pii-api-handler-staging` | `/staging` |
| push `main` | prod | `poco-pii-api-handler` | `/prod` |

**Do not push `main` until production is deliberately ready.**

AWS Academy Learner Lab constraints: `us-east-1` only, `LabRole` only (no custom IAM/OIDC), static credentials in GitHub secrets (refresh ~every 4 hours).

More detail: [`docs/ENVIRONMENTS.md`](docs/ENVIRONMENTS.md).

---

## API (website teammate)

Base (dev): `https://6xz841x652.execute-api.us-east-1.amazonaws.com/dev`

| Method | Path | Status |
|---|---|---|
| GET | `/health` | Ready |
| POST | `/encode` | Ready — `{ "text": "..." }` → redacted + mappings |
| POST | `/decode` | **501** — needs DynamoDB sessions |

Full contract: [`docs/API.md`](docs/API.md).

---

## Session / persistence (current)

Encode creates a **new vault per request**. Nothing is stored server-side yet.  
DynamoDB `pii-sessions-*` tables exist; wiring them is the next backend task.  
Until then the frontend should keep `mappings` in `localStorage` if it needs to survive a refresh.

---

## Known limitations

- Bare person names (no title/cue) are unreliable — Vault manual mapping is the workaround  
- PDF extract: CJK glyphs may be lost; line order may not match the logical document  
- Scanned/image-only PDFs are out of scope (`NoTextLayer`)  
- Card expiry/CVV and product codes/versions are intentionally not redacted  
- No BTC detector (ETH addresses are detected)

---

## Development

```bash
cd pii-tool
cargo test --workspace
cargo test -p pii-core --test gold_suite -- --nocapture   # print metrics
```

Lambda is built in CI inside `amazonlinux:2023` (glibc 2.34) with a small `__isoc23_str*` stub for ONNX Runtime. Do not build the deploy zip on a newer host glibc.

Tag: **`v0.2.0`** (algorithm complete + CI deploy green).
