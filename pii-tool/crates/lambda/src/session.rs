//! Session store for encode/decode mappings.
//! DynamoDB when `DYNAMODB_TABLE` is set (production); in-memory map for tests.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use aws_sdk_dynamodb::types::AttributeValue;
use aws_sdk_dynamodb::Client as DynamoClient;
use poco_core::vault::TokenMapping;

const TTL_SECS: i64 = 86_400; // 24h

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn table_name() -> Option<String> {
    std::env::var("DYNAMODB_TABLE")
        .ok()
        .filter(|s| !s.is_empty())
}

fn memory_maps() -> &'static Mutex<HashMap<String, Vec<(String, String, usize)>>> {
    static MEM: OnceLock<Mutex<HashMap<String, Vec<(String, String, usize)>>>> = OnceLock::new();
    MEM.get_or_init(|| {
        let inner = HashMap::new();
        let locked = Mutex::new(inner);
        locked
    })
}

fn to_row(m: &TokenMapping) -> (String, String, usize) {
    (m.class.clone(), m.value.clone(), m.first_offset)
}

fn from_row(class: String, value: String, first_offset: usize) -> TokenMapping {
    TokenMapping {
        token: class.clone(),
        value,
        class,
        first_offset,
    }
}

fn av_s(text: &str) -> AttributeValue {
    let owned = text.to_string();
    AttributeValue::S(owned)
}

fn av_n(value: i64) -> AttributeValue {
    let rendered = value.to_string();
    AttributeValue::N(rendered)
}

fn item_attr(item: &HashMap<String, AttributeValue>, key: &str) -> Option<String> {
    let found = item.get(key)?;
    match found.as_s() {
        Ok(text) => Some(text.to_string()),
        Err(_) => None,
    }
}

fn item_attr_usize(item: &HashMap<String, AttributeValue>, key: &str) -> usize {
    let Some(found) = item.get(key) else {
        return 0;
    };
    match found.as_n() {
        Ok(num) => num.parse::<usize>().unwrap_or(0),
        Err(_) => 0,
    }
}

pub async fn store_mappings(session_id: &str, mappings: &[TokenMapping]) -> Result<(), String> {
    let rows: Vec<(String, String, usize)> = mappings.iter().map(to_row).collect();
    let sid = session_id.to_string();
    match table_name() {
        None => {
            let maps = memory_maps();
            let mut mem = maps.lock().unwrap_or_else(|e| e.into_inner());
            mem.insert(sid, rows);
            Ok(())
        }
        Some(table) => {
            let config = aws_config::load_from_env().await;
            let client = DynamoClient::new(&config);
            let expires = now_epoch() + TTL_SECS;
            let key_session = "sessionId".to_string();
            let key_token = "token".to_string();
            let key_value = "value".to_string();
            let key_category = "category".to_string();
            let key_offset = "firstOffset".to_string();
            let key_expires = "expiresAt".to_string();
            for (class, value, first_offset) in rows {
                let mut item = HashMap::new();
                let sid_av = av_s(&sid);
                let k_session = key_session.clone();
                item.insert(k_session, sid_av);
                let token_av = av_s(&class);
                let k_token = key_token.clone();
                item.insert(k_token, token_av);
                let value_av = av_s(&value);
                let k_value = key_value.clone();
                item.insert(k_value, value_av);
                let cat = category_of(&class);
                let cat_av = av_s(&cat);
                let k_category = key_category.clone();
                item.insert(k_category, cat_av);
                let offset_av = av_n(first_offset as i64);
                let k_offset = key_offset.clone();
                item.insert(k_offset, offset_av);
                let exp_av = av_n(expires);
                let k_expires = key_expires.clone();
                item.insert(k_expires, exp_av);
                let stored = client
                    .put_item()
                    .table_name(&table)
                    .set_item(Some(item))
                    .send()
                    .await;
                if let Err(e) = stored {
                    let msg = format!("dynamodb put: {e}");
                    return Err(msg);
                }
            }
            Ok(())
        }
    }
}

pub async fn load_mappings(session_id: &str) -> Result<Vec<TokenMapping>, String> {
    let sid = session_id.to_string();
    match table_name() {
        None => {
            let maps = memory_maps();
            let mem = maps.lock().unwrap_or_else(|e| e.into_inner());
            let rows = mem.get(&sid).cloned().unwrap_or_default();
            Ok(rows
                .into_iter()
                .map(|(c, v, off)| from_row(c, v, off))
                .collect())
        }
        Some(table) => {
            let config = aws_config::load_from_env().await;
            let client = DynamoClient::new(&config);
            let sid_av = av_s(&sid);
            let resp = client
                .query()
                .table_name(&table)
                .key_condition_expression("sessionId = :s")
                .expression_attribute_values(":s", sid_av)
                .send()
                .await;
            let resp = match resp {
                Ok(r) => r,
                Err(e) => {
                    let msg = format!("dynamodb query: {e}");
                    return Err(msg);
                }
            };
            let mut out = Vec::new();
            for item in resp.items() {
                let class = item_attr(item, "token").unwrap_or_default();
                let value = item_attr(item, "value").unwrap_or_default();
                let first_offset = item_attr_usize(item, "firstOffset");
                if !class.is_empty() {
                    let row = from_row(class, value, first_offset);
                    out.push(row);
                }
            }
            Ok(out)
        }
    }
}

pub async fn delete_session(session_id: &str) -> Result<(), String> {
    let sid = session_id.to_string();
    match table_name() {
        None => {
            let maps = memory_maps();
            let mut mem = maps.lock().unwrap_or_else(|e| e.into_inner());
            mem.remove(&sid);
            Ok(())
        }
        Some(table) => {
            let config = aws_config::load_from_env().await;
            let client = DynamoClient::new(&config);
            let existing = load_mappings(session_id).await?;
            for m in existing {
                let sid_av = av_s(&sid);
                let tok_av = av_s(&m.class);
                let deleted = client
                    .delete_item()
                    .table_name(&table)
                    .key("sessionId", sid_av)
                    .key("token", tok_av)
                    .send()
                    .await;
                if let Err(e) = deleted {
                    let msg = format!("dynamodb delete: {e}");
                    return Err(msg);
                }
            }
            Ok(())
        }
    }
}

/// `Name_1` → `Name`; `Custom:date_1` → `Date`.
pub fn category_of(class: &str) -> String {
    let last = class.rsplit([':', '-']).next().unwrap_or(class);
    let base = last.rsplit('_').nth(1).unwrap_or(last);
    let mut chars = base.chars();
    match chars.next() {
        Some(c) => {
            let head = c.to_uppercase().collect::<String>();
            let tail = chars.as_str();
            head + tail
        }
        None => "Other".to_string(),
    }
}
