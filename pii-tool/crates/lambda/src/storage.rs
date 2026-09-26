//! Optional S3 archive for uploads / redacted text. No-op when `S3_BUCKET` is unset.

use aws_sdk_s3::operation::get_object::GetObjectOutput;
use aws_sdk_s3::operation::put_object::PutObjectOutput;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::presigning::PresigningConfig;
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

fn upload_key(file_name: &str, session_id: &str) -> String {
    s3_key("documents/uploads", file_name, session_id)
}

fn text_key(file_name: &str, session_id: &str) -> String {
    s3_key("documents/redacted", file_name, session_id)
}

/// Presigned PUT so the browser can upload large PDFs/DOCX directly to S3
/// (avoids API Gateway / Lambda ~6MB payload limit → HTTP 413).
pub async fn presign_upload(
    session_id: &str,
    file_name: &str,
    content_type: &str,
) -> Result<(String, String), String> {
    let Some(bucket) = bucket_name() else {
        let msg = "S3_BUCKET is not configured".to_string();
        return Err(msg);
    };
    let key = upload_key(file_name, session_id);
    let config = aws_config::load_from_env().await;
    let client = S3Client::new(&config);
    let duration = std::time::Duration::from_secs(900);
    let expires = PresigningConfig::expires_in(duration);
    let expires = match expires {
        Ok(e) => e,
        Err(e) => {
            let msg = format!("presign config: {e}");
            return Err(msg);
        }
    };
    let key_av = key.clone();
    let ct = content_type.to_string();
    let req = client
        .put_object()
        .bucket(&bucket)
        .key(&key_av)
        .content_type(ct)
        .presigned(expires)
        .await;
    let presigned = match req {
        Ok(p) => p,
        Err(e) => {
            let msg = format!("s3 presign: {e}");
            return Err(msg);
        }
    };
    let url = presigned.uri().to_string();
    Ok((key, url))
}

/// Read an object back (for extract-from-S3-key).
pub async fn get_bytes(key: &str) -> Result<Vec<u8>, String> {
    let Some(bucket) = bucket_name() else {
        let msg = "S3_BUCKET is not configured".to_string();
        return Err(msg);
    };
    let config = aws_config::load_from_env().await;
    let client = S3Client::new(&config);
    let key_owned = key.to_string();
    let got: Result<GetObjectOutput, _> = client.get_object().bucket(&bucket).key(key_owned).send().await;
    let out = match got {
        Ok(o) => o,
        Err(e) => {
            let msg = format!("s3 get: {e}");
            return Err(msg);
        }
    };
    let collected = out.body.collect().await;
    let bytes = match collected {
        Ok(b) => b.into_bytes(),
        Err(e) => {
            let msg = format!("s3 body: {e}");
            return Err(msg);
        }
    };
    Ok(bytes.to_vec())
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
    let key = upload_key(file_name, session_id);
    let config = aws_config::load_from_env().await;
    let client = S3Client::new(&config);
    let body = ByteStream::from(bytes);
    let key_av = key.clone();
    let stored: Result<PutObjectOutput, _> = client
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
    let key = text_key(file_name, session_id);
    let config = aws_config::load_from_env().await;
    let client = S3Client::new(&config);
    let bytes = text.as_bytes().to_vec();
    let body = ByteStream::from(bytes);
    let key_av = key.clone();
    let stored: Result<PutObjectOutput, _> = client
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
