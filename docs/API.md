# PII Vault API (v1) 

Runtime: AWS Lambda (Rust) behind API Gateway, **us-east-1**, AWS Academy Learner Lab.  
Session store: DynamoDB table `pii-sessions` (LabRole — no custom IAM).

All request/response bodies are JSON (`Content-Type: application/json`).  
All timestamps are **Unix epoch seconds** (numbers).  
Errors use HTTP status codes with a stable JSON body (see below).

```
https://{api-id}.execute-api.us-east-1.amazonaws.com/{stage}
```

Examples below use `https://api.example.com`.

---

## Session model

A **session** holds the encode/decode mapping for one document / chat turn.

| Field | Type | Notes |
|---|---|---|
| `sessionId` | string | Opaque id (UUID recommended). Optional on `/encode` — server generates one if omitted. Required on `/decode` and session routes. |
| `token` | string | Readable token class, e.g. `Email_1`, `Iban_1`, `Date_2`. |
| `value` | string | Original PII value. |
| `category` | string | Detector class family: `Email`, `Name`, `Phone`, `Date`, `Location`, `Ssn`, `Iban`, `CreditCard`, `IpAddress`, … |
| `firstOffset` | number | Byte offset of first occurrence in the source text (display sort key). |
| `expiresAt` | number | Epoch seconds. DynamoDB TTL (24h / 86400s from write). |

DynamoDB `pii-sessions-{env}` (table name from `DYNAMODB_TABLE`):

* Partition key: `sessionId` (S)
* Sort key: `token` (S)
* Attributes: `value` (S), `category` (S), `firstOffset` (N), `expiresAt` (N, TTL enabled)

Do **not** add extra session APIs without updating this doc.

---

## `GET /health`

Liveness. No auth, no body.

**200**

```json
{
  "status": "ok",
  "version": "0.1.0",
  "env": "dev",
  "path": "/dev/health"
}
```

---

## `POST /api/v1/encode`

Redact PII in free text and store mappings under `sessionId`.

**Request**

```json
{
  "sessionId": "3f2c9a1e-…",
  "text": "Contact Dr. Sarah Mitchell at s.mitchell@university.edu"
}
```

| Field | Type | Required | Notes |
|---|---|---|---|
| `sessionId` | string | no | Session id to store/merge mappings into. Omitted → server returns a new UUID. |
| `text` | string | yes | Raw document text (not file upload in v1). |

**200**

```json
{
  "sessionId": "3f2c9a1e-…",
  "redactedText": "Contact [Name_1] at [Email_1]",
  "mappings": [
    {
      "token": "Name_1",
      "value": "Dr. Sarah Mitchell",
      "category": "Name",
      "firstOffset": 8
    },
    {
      "token": "Email_1",
      "value": "s.mitchell@university.edu",
      "category": "Email",
      "firstOffset": 35
    }
  ],
  "stats": { "totalEntities": 2, "durationMs": 12 }
}
```

* `mappings` is sorted by **first appearance in the source text** (`firstOffset` ascending).
* Same `sessionId` reuses tokens for the same value (session accumulation).
* Website should render `redactedText` and show the mapping table in this order.

**400** missing/invalid `text` (or non-empty invalid JSON).

---

## `POST /api/v1/decode`

Restore PII in an LLM reply that contains readable tokens.

**Request**

```json
{
  "sessionId": "3f2c9a1e-…",
  "text": "I contacted [Name_1] and [Email_1] today."
}
```

**200**

```json
{
  "sessionId": "3f2c9a1e-…",
  "restoredText": "I contacted Dr. Sarah Mitchell and s.mitchell@university.edu today.",
  "hallucinations": ["[Person_99]"]
}
```

* Tokens in `[Class_N]` form are replaced from the session mapping.
* Unknown but token-shaped strings go in `hallucinations` and **stay** in `restoredText`.
* Non-token text is unchanged.

**400** missing fields. **404** unknown `sessionId` (optional; empty mapping is also valid → all tokens hallucinated).

---

## `GET /api/v1/sessions/{sessionId}/mappings`

List all mappings for a session, first-appearance order.

**200**

```json
{
  "sessionId": "3f2c9a1e-…",
  "mappings": [
    { "token": "Name_1", "value": "Dr. Sarah Mitchell", "category": "Name", "firstOffset": 8 },
    { "token": "Email_1", "value": "s.mitchell@university.edu", "category": "Email", "firstOffset": 35 }
  ]
}
```

Empty session → `"mappings": []`.

**400** missing `sessionId` path segment.

---

## `DELETE /api/v1/sessions/{sessionId}`

Clear the session vault (all tokens for that id).

**204** no body. Idempotent.

---

## Error envelope

Non-2xx responses:

```json
{
  "error": {
    "code": "bad_request",
    "message": "text is required"
  }
}
```

| HTTP | `code` | When |
|---|---|---|
| 400 | `bad_request` | Missing/invalid JSON fields |
| 404 | `not_found` | Unknown session (if enforced) |
| 405 | `method_not_allowed` | Wrong HTTP method |
| 500 | `internal` | Lambda/pipeline failure |

---

## CORS (website)

API Gateway must allow:

* `Origin`: your CloudFront/site origin (and `http://localhost:*` for dev)
* `Methods`: `GET`, `POST`, `DELETE`, `OPTIONS`
* `Headers`: `Content-Type`
* `Access-Control-Allow-Origin` echoed as appropriate (lab: `*` is acceptable if cookies are unused)

All calls are credential-less (no cookies). Treat `sessionId` as the only handle — do not put raw PII in query strings.

---

## Typical website flow

```text
1. User pastes text  →  POST /api/v1/encode
2. Show redactedText + mapping table (order = response.mappings)
3. User calls LLM with redactedText (out of band)
4. User pastes LLM reply  →  POST /api/v1/decode
5. Show restoredText + red hallucination list
6. Optional: GET mappings / DELETE session on "Clear"
```

### Example fetch

```js
const API = "https://api.example.com";

async function encode(sessionId, text) {
  const res = await fetch(`${API}/api/v1/encode`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ sessionId, text }),
  });
  if (!res.ok) throw new Error(await res.text());
  return res.json(); // { redactedText, mappings }
}

async function decode(sessionId, text) {
  const res = await fetch(`${API}/api/v1/decode`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ sessionId, text }),
  });
  if (!res.ok) throw new Error(await res.text());
  return res.json(); // { restoredText, hallucinations }
}
```

---

## AWS Academy Learner Lab notes (for deploy/integration)

* Region is **hardcoded `us-east-1`**.
* Lambda execution role is the pre-existing **`LabRole`** — never create IAM roles or OIDC providers.
* DynamoDB `pii-sessions` TTL on `expiresAt` = **86400 seconds** (24h).
* CI deploys with **static** `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` / `AWS_SESSION_TOKEN` GitHub secrets only.
