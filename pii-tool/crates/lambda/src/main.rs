//! AWS Lambda HTTP handler for pii-core (local code only — deploy is CI's job).
mod session;

use lambda_http::{run, service_fn, Body, Error, Request, Response};
use serde::Serialize;
use serde_json::json;
use std::time::Instant;

use poco_core::vault::{TokenMapping, Vault};
use session::{category_of, delete_session, load_mappings, store_mappings};

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

    if method == "POST" && segment == "encode" {
        let started = Instant::now();
        let parsed: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(_) => {
                let b = err_body("invalid json");
                return json_response(400, b);
            }
        };
        let text = match parsed.get("text").and_then(|t| t.as_str()) {
            Some(t) => t.to_string(),
            None => {
                let b = err_body("text is required");
                return json_response(400, b);
            }
        };
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
