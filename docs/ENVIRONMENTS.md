# Environments

Four isolated stacks in **us-east-1** (AWS Academy Learner Lab).  
One API Gateway: `poco-pii-api` / `6xz841x652`. Stages resolve each Lambda via stage variable `lambdaName`.

| Env | Git branch | Lambda | DynamoDB | S3 | API base |
|---|---|---|---|---|---|
| **dev** | `develop` | `poco-pii-api-handler-dev` | `pii-sessions-dev` | `poco-pii-tools-dev` | `…/dev` |
| **test** | `test/**` | `poco-pii-api-handler-test` | `pii-sessions-test` | `poco-pii-tools-test` | `…/test` |
| **staging** | `release/**` | `poco-pii-api-handler-staging` | `pii-sessions-staging` | `poco-pii-tools-staging` | `…/staging` |
| **prod** | `main` | `poco-pii-api-handler` | `pii-sessions` | `poco-pii-tools` | `…/prod` |

API ID: `6xz841x652` · Account: `830428004610`

---

## Shared function config

All Lambdas:

- **Runtime:** `provided.al2023` (custom bootstrap)
- **Handler:** `bootstrap`
- **Architecture:** `x86_64`
- **Role:** `LabRole` (cannot create custom IAM)
- **Memory / timeout:** 512 MB / 30s
- **Env vars:** `ENVIRONMENT`, `DYNAMODB_TABLE`, `S3_BUCKET`, `RUST_LOG=info`, `LD_LIBRARY_PATH=/var/task`

API Gateway `GET /health` must stay **Lambda proxy** (`AWS_PROXY`). Invoke permission `SourceArn` pattern: `arn:aws:execute-api:…:6xz841x652/{stage}/*/*`.

---

## dev (current primary)

- **Branch:** `develop`  
- **CI:** push → test job → AL2023 build → `update-function-code` → smoke `GET /dev/health`  
- **Verified live:**
  - `GET /dev/health` → 200 `{"status":"ok","version":"0.1.0","env":"dev"}`
  - `POST /dev/encode` → 200 redacted + mappings
  - `POST /dev/decode` → **501** (sessions not wired)
- **Use for:** website integration, day-to-day deploys

**Smoke (manual):**

```bash
curl -s https://6xz841x652.execute-api.us-east-1.amazonaws.com/dev/health
curl -s -X POST https://6xz841x652.execute-api.us-east-1.amazonaws.com/dev/encode \
  -H 'Content-Type: application/json' \
  -d '{"text":"Contact alice@example.com"}'
```

---

## test

- **Branch:** `test/**` (e.g. `test/qa`)  
- **Lambda:** `poco-pii-api-handler-test`  
- **Stage:** `/test`  
- **CI:** same pipeline as dev; smoke `GET /test/health`  
- **Use for:** QA / teammate demos on a separate URL so dev is not disturbed  

**Create the branch (after `test` ref is removed — see below):**

```bash
git push origin develop:test/qa
```

**Smoke:**

```bash
curl -s https://6xz841x652.execute-api.us-east-1.amazonaws.com/test/health
```

**Setup notes:**

- Code is already the same as dev; first push to `test/**` will deploy via CI  
- If CI cannot create `test/qa`, delete the old **`test`** branch in GitHub (it blocks `test/*` names)  

---

## staging

- **Branch:** `release/**` (current: **`release/v0.2.0`**)  
- **Lambda:** `poco-pii-api-handler-staging`  
- **Stage:** `/staging`  
- **CI:** same pipeline; smoke `GET /staging/health`  
- **Use for:** pre-prod sign-off, group demos  

**Smoke:**

```bash
curl -s https://6xz841x652.execute-api.us-east-1.amazonaws.com/staging/health
```

**Setup notes:**

- `release/v0.2.0` is cut from `develop` at tag `v0.2.0`  
- Old `release/v0.1.0` is obsolete — delete it in GitHub when convenient  
- Staging already has a working binary (same build as dev)  

---

## prod

- **Branch:** `main` only — **do not push until the team is ready**  
- **Lambda:** `poco-pii-api-handler`  
- **Stage:** `/prod`  

**Smoke (when ready):**

```bash
curl -s https://6xz841x652.execute-api.us-east-1.amazonaws.com/prod/health
```

**Rules:**

- No experimental merges to `main`  
- Promote from staging after smoke + review  
- Credentials expire every ~4h — refresh GitHub secrets before any deploy  

---

## GitHub Actions

Workflow: `.github/workflows/deploy.yml` (`Deploy`)

1. `test` — `cargo test --workspace`  
2. `deploy`  
   - Map branch → env + Lambda name  
   - Build in `amazonlinux:2023` (`pii-tool/crates/lambda/build_lambda.sh`)  
   - glibc 2.38 stub for ONNX (`glibc_compat.c`)  
   - `update-function-code`  
   - Smoke `GET {API}/{stage}/health`  

**Secrets:** `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN` (Lab static keys; rotate when the lab expires).

---

## CloudWatch

Alarms (Errors ≥ 1 in 5 min):

- `poco-pii-api-handler-dev-errors`  
- `poco-pii-api-handler-test-errors`  
- `poco-pii-api-handler-staging-errors`  
- `poco-pii-api-handler-errors`  

Log groups: `/aws/lambda/poco-pii-api-handler-*`

---

## Environments not fully “CI-wired” yet

| Item | Status | Action |
|---|---|---|
| `test/qa` branch | Blocked by old `test` ref | Delete `test` in GitHub, then `git push origin develop:test/qa` |
| `release/v0.1.0` | Obsolete | Delete in GitHub (this session cannot delete remote branches) |
| Copilot as repo contributor | To remove | GitHub → Settings → Collaborators / Apps → remove Copilot |
| `/decode` + DynamoDB sessions | Not implemented | Next backend task |

---

## Checklist before using a stage for demos

1. `GET /{stage}/health` returns 200  
2. `POST /{stage}/encode` returns redacted + mappings  
3. CI `Deploy` workflow is green for that branch  
4. GitHub secrets are from a **live** Learner Lab session  
5. Do not paste real secrets into the frontend repo  
