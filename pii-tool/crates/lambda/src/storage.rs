//! Optional S3 archive for uploads / redacted text. No-op when `S3_BUCKET` is unset.

use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client as S3Client;

fn bucket_name() -> Option<String> {
    std::env::var("S3_BUCKET")
        .ok()
        .filter(|s| !s.is_empty())
}

fn s3_key(prefix: &str, file_name: &str, session_id: &str) -> String {
    let safe: String = file_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{prefix}/{session_id}/{safe}")
}

/// Store original upload bytes under `documents/uploads/`.
pub async fn put_upload(
    session_id: &str,
    file_name: &str,
    bytes: Vec<u8>,
) -> Result<Option<String>, String> {
    let Some(bucket) = bucket_name() else {
        return Ok(None);
    };
    let key = s3_key("documents/uploads", file_name, session_id);
    let config = aws_config::load_from_env().await;
    let client = S3Client::new(&config);
    let body = ByteStream::from(bytes);
    let key_av = key.clone();
    let stored = client
        .put_object()
        .bucket(&bucket)
        .key(&key_av)
        .body(body)
        .content_type("application/octet-stream")
        .send()
        .await;
    if let Err(e) = stored {
        let msg = format!("s3 put upload: {e}");
        return Err(msg);
    }
    Ok(Some(key))
}

/// Store redacted / extracted text under `documents/redacted/`.
pub async fn put_text(
    session_id: &str,
    file_name: &str,
    text: &str,
) -> Result<Option<String>, String> {
    let Some(bucket) = bucket_name() else {
        return Ok(None);
    };
    let key = s3_key("documents/redacted", file_name, session_id);
    let config = aws_config::load_from_env().await;
    let client = S3Client::new(&config);
    let bytes = text.as_bytes().to_vec();
    let body = ByteStream::from(bytes);
    let key_av = key.clone();
    let stored = client
        .put_object()
        .bucket(&bucket)
        .key(&key_av)
        .body(body)
        .content_type("text/plain; charset=utf-8")
        .send()
        .await;
    if let Err(e) = stored {
        let msg = format!("s3 put text: {e}");
        return Err(msg);
    }
    Ok(Some(key))
}
