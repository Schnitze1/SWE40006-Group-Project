//! AWS Lambda HTTP handler for pii-core (local code only — deploy is CI's job).
mod session;

use lambda_http::{run, service_fn, Body, Error, Request, Response};
use serde::Serialize;
use serde_json::json;
use std::time::Instant;

use poco_core::vault::{TokenMapping, Vault};
use session::{category_of, delete_mapping, delete_session, load_mappings, store_mappings, upsert_mapping};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MappingOut {
    token: String,
    value: String,
    category: String,
    first_offset: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatsOut {
    total_entities: usize,
    duration_ms: u128,
}

fn cors() -> [(&'static str, &'static str); 3] {
    [
        ("Access-Control-Allow-Origin", "*"),
        ("Access-Control-Allow-Headers", "Content-Type,Authorization"),
        ("Access-Control-Allow-Methods", "GET,POST,DELETE,OPTIONS"),
    ]
}

fn json_response(status: u16, body: String) -> Response<String> {
    let mut builder = Response::builder().status(status);
    for (k, v) in cors() {
        builder = builder.header(k, v);
    }
    let result = builder.header("Content-Type", "application/json").body(body);
    match result {
        Ok(r) => r,
        Err(_) => {
            let fallback = String::from("{\"error\":\"internal\"}");
            Response::new(fallback)
        }
    }
}

fn empty_response(status: u16) -> Response<String> {
    let empty = String::new();
    json_response(status, empty)
}

fn err_body(msg: &str) -> String {
    let value = json!({ "error": msg });
    value.to_string()
}

/// Minimal standard-base64 encoder for tests and request building.
#[cfg(test)]
fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(n >> 6) as usize & 63] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[n as usize & 63] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Minimal standard-base64 decoder (uploads from the web UI).
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut rev = [i8::MAX; 256];
    let mut i = 0;
    while i < 64 {
        rev[TABLE[i] as usize] = i as i8;
        i += 1;
    }
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits = 0;
    for c in input.bytes() {
        if c == b'=' || c == b'\n' || c == b'\r' || c == b' ' {
            continue;
        }
        let val = rev[c as usize];
        if val == i8::MAX {
            return None;
        }
        buf = (buf << 6) | (val as u32 & 0x3f);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

fn mapping_out(m: &TokenMapping) -> MappingOut {
    MappingOut {
        token: m.class.clone(),
        value: m.value.clone(),
        category: category_of(&m.class),
        first_offset: m.first_offset,
    }
}

fn session_id_from_path(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let idx = parts.iter().position(|p| *p == "sessions")?;
    let id = parts.get(idx + 1)?;
    if id.is_empty() {
        None
    } else {
        Some((*id).to_string())
    }
}

/// Token from `/sessions/{id}/mappings/{token}` — last path segment.
fn token_from_path(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let idx = parts.iter().position(|p| *p == "mappings")?;
    let tok = parts.get(idx + 1)?;
    if tok.is_empty() {
        None
    } else {
        Some((*tok).to_string())
    }
}

/// Pure-ish routing (session I/O is in-process or DynamoDB).
pub async fn route(method: &str, path: &str, body: &str) -> Response<String> {
    let method = method.to_uppercase();
    let path = path.split('?').next().unwrap_or(path);
    // API Gateway may include the stage prefix (/dev/encode). Match the last segment.
    let segment = path.rsplit('/').find(|s| !s.is_empty()).unwrap_or("");

    if method == "OPTIONS" {
        return empty_response(200);
    }

    if method == "GET" && segment == "health" {
        let env = std::env::var("ENVIRONMENT").unwrap_or_else(|_| "dev".to_string());
        let version = env!("CARGO_PKG_VERSION");
        let payload = json!({
            "status": "ok",
            "version": version,
            "env": env,
            "path": path
        });
        let body = payload.to_string();
        return json_response(200, body);
    }

    if method == "POST" && segment == "extract" {
        let parsed: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(_) => {
                let b = err_body("invalid json");
                return json_response(400, b);
            }
        };
        let file_name = match parsed.get("fileName").and_then(|t| t.as_str()) {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => {
                let b = err_body("fileName is required");
                return json_response(400, b);
            }
        };
        let b64 = match parsed.get("fileBase64").and_then(|t| t.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => {
                let b = err_body("fileBase64 is required");
                return json_response(400, b);
            }
        };
        let bytes = match base64_decode(&b64) {
            Some(b) => b,
            None => {
                let b = err_body("fileBase64 is not valid base64");
                return json_response(400, b);
            }
        };
        if bytes.is_empty() {
            let b = err_body("uploaded file is empty");
            return json_response(400, b);
        }
        let text = match poco_core::extract::extract_text_from_bytes(&file_name, &bytes) {
            Ok(t) => t,
            Err(e) => {
                let msg = format!("{e:?}");
                let b = err_body(&msg);
                return json_response(400, b);
            }
        };
        let out = json!({ "fileName": file_name, "text": text });
        let body = out.to_string();
        return json_response(200, body);
    }

    if method == "POST" && segment == "encode" {
        let started = Instant::now();
        let parsed: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(_) => {
                let b = err_body("invalid json");
                return json_response(400, b);
            }
        };
        let mut text = match parsed.get("text").and_then(|t| t.as_str()) {
            Some(t) => t.to_string(),
            None => String::new(),
        };
        if text.is_empty() {
            let file_name = parsed.get("fileName").and_then(|t| t.as_str()).unwrap_or("");
            let b64 = parsed.get("fileBase64").and_then(|t| t.as_str()).unwrap_or("");
            if !file_name.is_empty() && !b64.is_empty() {
                let decoded = base64_decode(b64);
                let bytes = match decoded {
                    Some(b) if !b.is_empty() => b,
                    _ => {
                        let b = err_body("fileBase64 is not valid base64");
                        return json_response(400, b);
                    }
                };
                let extracted = poco_core::extract::extract_text_from_bytes(file_name, &bytes);
                text = match extracted {
                    Ok(t) => t,
                    Err(e) => {
                        let msg = format!("{e:?}");
                        let b = err_body(&msg);
                        return json_response(400, b);
                    }
                };
            }
        }
        if text.is_empty() {
            let b = err_body("text or fileBase64 is required");
            return json_response(400, b);
        }
        let session_id = parsed
            .get("sessionId")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let vault = match Vault::new() {
            Ok(v) => v,
            Err(e) => {
                let msg = format!("{e:?}");
                let b = err_body(&msg);
                return json_response(500, b);
            }
        };
        let encoded = match vault.encode(&text) {
            Ok(e) => e,
            Err(e) => {
                let msg = format!("{e:?}");
                let b = err_body(&msg);
                return json_response(500, b);
            }
        };
        if let Err(e) = store_mappings(&session_id, &encoded.mappings).await {
            let b = err_body(&e);
            return json_response(500, b);
        }
        let mappings: Vec<MappingOut> = encoded.mappings.iter().map(mapping_out).collect();
        let total_entities = mappings.len();
        let duration_ms = started.elapsed().as_millis();
        let stats = StatsOut {
            total_entities,
            duration_ms,
        };
        let out = json!({
            "sessionId": session_id,
            "redactedText": encoded.redacted_text,
            "mappings": mappings,
            "stats": stats
        });
        let body = out.to_string();
        return json_response(200, body);
    }

    if method == "POST" && segment == "decode" {
        let parsed: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(_) => {
                let b = err_body("invalid json");
                return json_response(400, b);
            }
        };
        let session_id = match parsed.get("sessionId").and_then(|t| t.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => {
                let b = err_body("sessionId is required");
                return json_response(400, b);
            }
        };
        let text = parsed
            .get("text")
            .or_else(|| parsed.get("llmResponse"))
            .and_then(|t| t.as_str())
            .map(|s| s.to_string());
        let text = match text {
            Some(t) if !t.is_empty() => t,
            _ => {
                let b = err_body("text is required");
                return json_response(400, b);
            }
        };
        let mappings = match load_mappings(&session_id).await {
            Ok(m) => m,
            Err(e) => {
                let b = err_body(&e);
                return json_response(500, b);
            }
        };
        let vault = match Vault::new() {
            Ok(v) => v,
            Err(e) => {
                let msg = format!("{e:?}");
                let b = err_body(&msg);
                return json_response(500, b);
            }
        };
        let decoded = match vault.decode(&text, &mappings) {
            Ok(d) => d,
            Err(e) => {
                let msg = format!("{e:?}");
                let b = err_body(&msg);
                return json_response(500, b);
            }
        };
        let out = json!({
            "sessionId": session_id,
            "restoredText": decoded.restored_text,
            "hallucinations": decoded.hallucinations
        });
        let body = out.to_string();
        return json_response(200, body);
    }

    if method == "GET" && segment == "mappings" {
        let session_id = match session_id_from_path(path) {
            Some(s) => s,
            None => {
                let b = err_body("sessionId is required");
                return json_response(400, b);
            }
        };
        let mappings = match load_mappings(&session_id).await {
            Ok(m) => m,
            Err(e) => {
                let b = err_body(&e);
                return json_response(500, b);
            }
        };
        let list: Vec<MappingOut> = mappings.iter().map(mapping_out).collect();
        let out = json!({ "sessionId": session_id, "mappings": list });
        let body = out.to_string();
        return json_response(200, body);
    }

    if method == "PUT" && segment == "mappings" {
        let session_id = match session_id_from_path(path) {
            Some(s) => s,
            None => {
                let b = err_body("sessionId is required");
                return json_response(400, b);
            }
        };
        let parsed: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(_) => {
                let b = err_body("invalid json");
                return json_response(400, b);
            }
        };
        let token = match parsed.get("token").and_then(|t| t.as_str()) {
            Some(t) if !t.is_empty() => t.to_string(),
            _ => {
                let b = err_body("token is required");
                return json_response(400, b);
            }
        };
        let value = parsed
            .get("value")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        let category = parsed
            .get("category")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        if let Err(e) = upsert_mapping(&session_id, &token, &value, &category).await {
            let b = err_body(&e);
            return json_response(500, b);
        }
        let out = json!({
            "sessionId": session_id,
            "mapping": {
                "token": token,
                "value": value,
                "category": if category.is_empty() { category_of(&token) } else { category }
            }
        });
        let body = out.to_string();
        return json_response(200, body);
    }

    if method == "DELETE" && path.contains("/mappings/") {
        let session_id = match session_id_from_path(path) {
            Some(s) => s,
            None => {
                let b = err_body("sessionId is required");
                return json_response(400, b);
            }
        };
        let token = match token_from_path(path) {
            Some(t) => t,
            None => {
                let b = err_body("token is required");
                return json_response(400, b);
            }
        };
        if let Err(e) = delete_mapping(&session_id, &token).await {
            let b = err_body(&e);
            return json_response(500, b);
        }
        return empty_response(204);
    }

    if method == "DELETE" && path.contains("/sessions/") {
        let session_id = match session_id_from_path(path) {
            Some(s) => s,
            None => {
                let b = err_body("sessionId is required");
                return json_response(400, b);
            }
        };
        if let Err(e) = delete_session(&session_id).await {
            let b = err_body(&e);
            return json_response(500, b);
        }
        return empty_response(204);
    }

    let b = err_body("not found");
    json_response(404, b)
}

async fn handler(request: Request) -> Result<Response<Body>, Error> {
    let method = request.method().as_str().to_string();
    let path = request.uri().path().to_string();
    let body = match request.body() {
        Body::Text(s) => s.clone(),
        Body::Binary(b) => String::from_utf8_lossy(b).to_string(),
        Body::Empty => String::new(),
    };
    let resp = route(&method, &path, &body).await;
    let (parts, text) = resp.into_parts();
    let mut out = Response::builder().status(parts.status);
    for (k, v) in parts.headers.iter() {
        out = out.header(k, v);
    }
    let built = out.body(Body::Text(text));
    match built {
        Ok(r) => Ok(r),
        Err(_) => Ok(Response::new(Body::Empty)),
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let future = service_fn(handler);
    run(future).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_ok() {
        let resp = route("GET", "/health", "").await;
        assert_eq!(resp.status().as_u16(), 200);
        let body = resp.body();
        assert!(body.contains("\"status\":\"ok\""));
        assert!(body.contains("\"version\""));
    }

    #[tokio::test]
    async fn encode_redacts_email() {
        let resp = route("POST", "/encode", r#"{"text":"Contact alice@example.com"}"#).await;
        assert_eq!(resp.status().as_u16(), 200);
        let body = resp.body();
        assert!(body.contains("[Email_1]"), "body: {body}");
        assert!(body.contains("sessionId"), "body: {body}");
    }

    #[tokio::test]
    async fn encode_decode_round_trip_with_session() {
        let sid = "test-roundtrip-1";
        let enc = route(
            "POST",
            "/encode",
            &format!(r#"{{"sessionId":"{sid}","text":"Contact alice@example.com"}}"#),
        )
        .await;
        assert_eq!(enc.status().as_u16(), 200);
        let enc_body = enc.body().clone();
        assert!(enc_body.contains("sessionId"), "body: {enc_body}");

        let dec = route(
            "POST",
            "/decode",
            &format!(r#"{{"sessionId":"{sid}","text":"I emailed [Email_1]"}}"#),
        )
        .await;
        assert_eq!(dec.status().as_u16(), 200);
        let dec_body = dec.body();
        assert!(
            dec_body.contains("alice@example.com"),
            "decode restore failed: {dec_body}"
        );
    }

    #[tokio::test]
    async fn decode_hallucinated_token() {
        let sid = "test-halluc-1";
        route(
            "POST",
            "/encode",
            &format!(r#"{{"sessionId":"{sid}","text":"Contact alice@example.com"}}"#),
        )
        .await;
        let dec = route(
            "POST",
            "/decode",
            &format!(r#"{{"sessionId":"{sid}","text":"see [Person_99]"}}"#),
        )
        .await;
        assert_eq!(dec.status().as_u16(), 200);
        let body = dec.body();
        assert!(body.contains("Person_99"), "body: {body}");
        assert!(body.contains("hallucinations"), "body: {body}");
    }

    #[tokio::test]
    async fn mappings_listed_for_session() {
        let sid = "test-mappings-1";
        route(
            "POST",
            "/encode",
            &format!(r#"{{"sessionId":"{sid}","text":"Contact alice@example.com"}}"#),
        )
        .await;
        let get = route("GET", &format!("/api/v1/sessions/{sid}/mappings"), "").await;
        assert_eq!(get.status().as_u16(), 200);
        assert!(get.body().contains("alice@example.com"), "{}", get.body());
    }

    #[tokio::test]
    async fn mapping_update_changes_decode() {
        let sid = "test-map-update-1";
        route(
            "POST",
            "/encode",
            &format!(r#"{{"sessionId":"{sid}","text":"Contact alice@example.com"}}"#),
        )
        .await;
        let put = route(
            "PUT",
            &format!("/api/v1/sessions/{sid}/mappings"),
            r#"{"token":"Email_1","value":"bob@corp.com","category":"Email"}"#,
        )
        .await;
        assert_eq!(put.status().as_u16(), 200);
        assert!(put.body().contains("bob@corp.com"), "{}", put.body());
        let dec = route(
            "POST",
            "/decode",
            &format!(r#"{{"sessionId":"{sid}","text":"see [Email_1]"}}"#),
        )
        .await;
        assert_eq!(dec.status().as_u16(), 200);
        assert!(dec.body().contains("bob@corp.com"), "{}", dec.body());
    }

    #[tokio::test]
    async fn mapping_delete_makes_token_hallucination() {
        let sid = "test-map-del-1";
        route(
            "POST",
            "/encode",
            &format!(r#"{{"sessionId":"{sid}","text":"Contact alice@example.com"}}"#),
        )
        .await;
        let del = route(
            "DELETE",
            &format!("/api/v1/sessions/{sid}/mappings/Email_1"),
            "",
        )
        .await;
        assert_eq!(del.status().as_u16(), 204);
        let dec = route(
            "POST",
            "/decode",
            &format!(r#"{{"sessionId":"{sid}","text":"see [Email_1]"}}"#),
        )
        .await;
        assert_eq!(dec.status().as_u16(), 200);
        let body = dec.body();
        assert!(body.contains("Email_1"), "body: {body}");
        assert!(body.contains("hallucinations"), "body: {body}");
    }

    #[tokio::test]
    async fn extract_rejects_missing_file() {
        let resp = route("POST", "/extract", r#"{"fileName":"a.pdf"}"#).await;
        assert_eq!(resp.status().as_u16(), 400);
        assert!(resp.body().contains("fileBase64"));
    }

    #[tokio::test]
    async fn encode_accepts_uploaded_txt_file() {
        let raw = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test_data/onboarding_dossier.txt"
        ))
        .expect("fixture txt");
        let b64 = base64_encode(&raw);
        let payload = format!(
            r#"{{"fileName":"onboarding_dossier.txt","fileBase64":"{b64}"}}"#
        );
        let resp = route("POST", "/encode", &payload).await;
        assert_eq!(resp.status().as_u16(), 200);
        let body = resp.body();
        assert!(body.contains("[Email_") || body.contains("Email"), "body: {body}");
    }

    #[tokio::test]
    async fn mapping_put_requires_token() {
        let sid = "test-map-bad-1";
        let put = route(
            "PUT",
            &format!("/api/v1/sessions/{sid}/mappings"),
            r#"{"value":"x"}"#,
        )
        .await;
        assert_eq!(put.status().as_u16(), 400);
        assert!(put.body().contains("token is required"));
    }

    #[tokio::test]
    async fn delete_session_is_idempotent() {
        let sid = "test-delete-1";
        route(
            "POST",
            "/encode",
            &format!(r#"{{"sessionId":"{sid}","text":"Contact alice@example.com"}}"#),
        )
        .await;
        let d1 = route("DELETE", &format!("/api/v1/sessions/{sid}"), "").await;
        assert_eq!(d1.status().as_u16(), 204);
        let d2 = route("DELETE", &format!("/api/v1/sessions/{sid}"), "").await;
        assert_eq!(d2.status().as_u16(), 204);
    }

    #[tokio::test]
    async fn encode_malformed_json_400() {
        let resp = route("POST", "/encode", "{not json").await;
        assert_eq!(resp.status().as_u16(), 400);
        assert!(resp.body().contains("invalid json"));
    }

    #[tokio::test]
    async fn unknown_path_404() {
        let resp = route("GET", "/nope", "").await;
        assert_eq!(resp.status().as_u16(), 404);
        assert!(resp.body().contains("not found"));
    }

    #[tokio::test]
    async fn cors_on_every_response() {
        let cases = [
            route("GET", "/health", "").await,
            route("POST", "/encode", "{}").await,
            route("POST", "/decode", "{}").await,
            route("GET", "/missing", "").await,
            route("OPTIONS", "/encode", "").await,
        ];
        for resp in cases {
            let headers = resp.headers();
            let origin = headers
                .get("Access-Control-Allow-Origin")
                .map(|v| v.to_str().unwrap_or(""));
            assert_eq!(origin, Some("*"));
            assert!(headers.contains_key("Access-Control-Allow-Headers"));
            assert!(headers.contains_key("Access-Control-Allow-Methods"));
        }
    }

    #[tokio::test]
    async fn encode_works_with_stage_prefix() {
        let resp = route(
            "POST",
            "/dev/encode",
            r#"{"text":"Contact alice@example.com"}"#,
        )
        .await;
        assert_eq!(resp.status().as_u16(), 200);
        assert!(resp.body().contains("[Email_1]"), "body: {}", resp.body());
    }

    #[tokio::test]
    async fn health_works_with_stage_prefix() {
        let resp = route("GET", "/staging/health", "").await;
        assert_eq!(resp.status().as_u16(), 200);
        assert!(resp.body().contains("\"status\":\"ok\""));
    }
}
