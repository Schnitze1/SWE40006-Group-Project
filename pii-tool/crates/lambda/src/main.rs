//! AWS Lambda HTTP handler for pii-core (local code only — deploy is CI's job).
use lambda_http::{run, service_fn, Body, Error, Request, Response};
use serde::Serialize;
use serde_json::json;
use std::time::Instant;

use poco_core::vault::Vault;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MappingOut {
    token: String,
    value: String,
    category: String,
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
        ("Access-Control-Allow-Methods", "GET,POST,OPTIONS"),
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

fn err_body(msg: &str) -> String {
    let value = json!({ "error": msg });
    value.to_string()
}

/// Pure routing for local tests. `body` is the raw request body (JSON for POST).
pub fn route(method: &str, path: &str, body: &str) -> Response<String> {
    let method = method.to_uppercase();
    let path = path.split('?').next().unwrap_or(path);

    if method == "OPTIONS" {
        let empty = String::new();
        return json_response(200, empty);
    }
    if method == "GET" && path == "/health" {
        let env = std::env::var("ENVIRONMENT").unwrap_or_else(|_| "dev".to_string());
        let version = env!("CARGO_PKG_VERSION");
        let payload = json!({
            "status": "ok",
            "version": version,
            "env": env
        });
        let body = payload.to_string();
        return json_response(200, body);
    }
    if method == "POST" && path == "/encode" {
        let started = Instant::now();
        let parsed: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(_) => {
                let body = err_body("invalid json");
                return json_response(400, body);
            }
        };
        let text = match parsed.get("text").and_then(|t| t.as_str()) {
            Some(t) => t.to_string(),
            None => {
                let body = err_body("invalid json");
                return json_response(400, body);
            }
        };
        let vault = match Vault::new() {
            Ok(v) => v,
            Err(e) => {
                let msg = format!("{e:?}");
                let body = err_body(&msg);
                return json_response(500, body);
            }
        };
        let encoded = match vault.encode(&text) {
            Ok(e) => e,
            Err(e) => {
                let msg = format!("{e:?}");
                let body = err_body(&msg);
                return json_response(500, body);
            }
        };
        let mappings: Vec<MappingOut> = encoded
            .mappings
            .iter()
            .map(|m| MappingOut {
                token: format!("[{}]", m.class),
                value: m.value.clone(),
                category: m.class.clone(),
            })
            .collect();
        let total_entities = mappings.len();
        let duration_ms = started.elapsed().as_millis();
        let stats = StatsOut {
            total_entities,
            duration_ms,
        };
        let out = json!({
            "redactedText": encoded.redacted_text,
            "mappings": mappings,
            "stats": stats
        });
        let body = out.to_string();
        return json_response(200, body);
    }
    if method == "POST" && path == "/decode" {
        let body = err_body("decode not yet wired to DynamoDB sessions");
        return json_response(501, body);
    }
    let body = err_body("not found");
    json_response(404, body)
}

async fn handler(request: Request) -> Result<Response<Body>, Error> {
    let method = request.method().as_str().to_string();
    let path = request.uri().path().to_string();
    let body = match request.body() {
        Body::Text(s) => s.clone(),
        Body::Binary(b) => String::from_utf8_lossy(b).to_string(),
        Body::Empty => String::new(),
    };
    let resp = route(&method, &path, &body);
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

    #[test]
    fn health_ok() {
        let resp = route("GET", "/health", "");
        assert_eq!(resp.status().as_u16(), 200);
        let body = resp.body();
        assert!(body.contains("\"status\":\"ok\""));
        assert!(body.contains("\"version\""));
    }

    #[test]
    fn encode_redacts_email() {
        let resp = route("POST", "/encode", r#"{"text":"Contact alice@example.com"}"#);
        assert_eq!(resp.status().as_u16(), 200);
        let body = resp.body();
        assert!(body.contains("[Email_1]"), "body: {body}");
    }

    #[test]
    fn encode_malformed_json_400() {
        let resp = route("POST", "/encode", "{not json");
        assert_eq!(resp.status().as_u16(), 400);
        assert!(resp.body().contains("invalid json"));
    }

    #[test]
    fn unknown_path_404() {
        let resp = route("GET", "/nope", "");
        assert_eq!(resp.status().as_u16(), 404);
        assert!(resp.body().contains("not found"));
    }

    #[test]
    fn cors_on_every_response() {
        let cases = [
            route("GET", "/health", ""),
            route("POST", "/encode", "{}"),
            route("POST", "/decode", "{}"),
            route("GET", "/missing", ""),
            route("OPTIONS", "/encode", ""),
        ];
        for resp in cases {
            let headers = resp.headers();
            let origin = headers.get("Access-Control-Allow-Origin").map(|v| v.to_str().unwrap_or(""));
            assert_eq!(origin, Some("*"));
            assert!(headers.contains_key("Access-Control-Allow-Headers"));
            assert!(headers.contains_key("Access-Control-Allow-Methods"));
        }
    }

    #[test]
    fn decode_stub_501() {
        let resp = route("POST", "/decode", r#"{"llmResponse":"hi"}"#);
        assert_eq!(resp.status().as_u16(), 501);
        assert!(resp.body().contains("DynamoDB"));
    }
}
