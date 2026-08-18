//! # cells/api.rs
//!
//! API cell evaluator.
//!
//! ## Role in the system
//!
//! Async, may have effects (network, model), may be expensive. Caller
//! context can route which model/endpoint to use. The endpoint can be:
//!
//! - an HTTP URL (with `{{caller.row}}`-style template substitution)
//! - a `model:foo` pseudo-URL (placeholder; a real implementation would
//!   look up the provider, swap based on context, call the model API)
//! - an `mcp://server/tool` reference (placeholder)
//!
//! ## Depends on
//!
//! - `reqwest` — async HTTP client.
//! - `crate::types` — `Cell`, `CellValue`, `CallerContext`, `Effect`.
//!
//! ## Used by
//!
//! - `crate::engine` — dispatches to this on `get`/`call` for `api`
//!   cells.
//!
//! ## Key decisions
//!
//! - We accept a pluggable `ApiExecutor` so tests can inject a fake
//!   without a real network round-trip. The default executor uses
//!   `reqwest::Client`.
//! - Substitutions are `{{path.to.value}}` walks into the `CallerContext`,
//!   matching the TypeScript original. Missing values become empty
//!   strings.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::Response;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::types::{now_millis, Cell, CellStatus, CellValue, Effect};

/// Pluggable HTTP transport. Tests use the `Stub` variant to avoid the
/// network; production uses `Reqwest`.
#[async_trait]
pub trait ApiExecutor: Send + Sync {
    /// Issue an HTTP request and return the response.
    async fn execute(
        &self,
        method: &str,
        url: &str,
        headers: &std::collections::BTreeMap<String, String>,
        body: Option<&str>,
    ) -> Result<ApiResponse>;
}

/// A stripped-down HTTP response — just what the cell needs.
#[derive(Debug, Clone)]
pub struct ApiResponse {
    /// The HTTP status code.
    pub status: u16,
    /// The reason phrase (e.g. `"OK"`).
    pub status_text: String,
    /// Response headers, lowercased keys.
    pub headers: std::collections::BTreeMap<String, String>,
    /// The body, as a JSON value if the content type was JSON; otherwise
    /// as a JSON string.
    pub body: Value,
}

/// The default executor. Wraps `reqwest::Client`.
pub struct ReqwestExecutor {
    /// The underlying client. Cloned for each call (reqwest clients are
    /// cheap to clone and internally share a connection pool).
    client: reqwest::Client,
}

impl ReqwestExecutor {
    /// Create a new executor with a default-configured client.
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client should build with default config");
        Self { client }
    }
}

impl Default for ReqwestExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ApiExecutor for ReqwestExecutor {
    async fn execute(
        &self,
        method: &str,
        url: &str,
        headers: &std::collections::BTreeMap<String, String>,
        body: Option<&str>,
    ) -> Result<ApiResponse> {
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|e| Error::Config(format!("invalid HTTP method '{method}': {e}")))?;
        let mut req = self.client.request(method, url);
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        if let Some(b) = body {
            req = req.body(b.to_string());
        }
        let resp = req.send().await?;
        Ok(response_to_apiresponse(resp).await?)
    }
}

async fn response_to_apiresponse(resp: Response) -> Result<ApiResponse> {
    let status = resp.status();
    let status_code = status.as_u16();
    let status_text = status
        .canonical_reason()
        .unwrap_or("")
        .to_string();
    let mut headers = std::collections::BTreeMap::new();
    for (k, v) in resp.headers().iter() {
        headers.insert(k.as_str().to_lowercase(), v.to_str().unwrap_or("").to_string());
    }
    let content_type = headers
        .get("content-type")
        .cloned()
        .unwrap_or_default();
    let body_text = resp.text().await?;
    let body = if content_type.contains("application/json") {
        serde_json::from_str(&body_text).unwrap_or(Value::String(body_text))
    } else {
        Value::String(body_text)
    };
    Ok(ApiResponse {
        status: status_code,
        status_text,
        headers,
        body,
    })
}

/// A test executor. Returns a canned response regardless of the request.
pub struct StubExecutor {
    /// The response to return.
    pub response: ApiResponse,
}

#[async_trait]
impl ApiExecutor for StubExecutor {
    async fn execute(
        &self,
        _method: &str,
        _url: &str,
        _headers: &std::collections::BTreeMap<String, String>,
        _body: Option<&str>,
    ) -> Result<ApiResponse> {
        Ok(self.response.clone())
    }
}

/// Convenience alias used in `CellDef`-shaped contexts. The full type is
/// `Arc<dyn ApiExecutor>`.
pub type ApiExecutorRef = std::sync::Arc<dyn ApiExecutor>;

/// Evaluate an API cell.
///
/// The `executor` parameter lets tests substitute a stub. Pass `None`
/// to use the default reqwest-based executor.
///
/// Takes the `Cell` by value so the returned future is `Send` and
/// can be moved across thread boundaries by `drive_async`.
pub async fn evaluate_api(
    cell: Cell,
    ctx: crate::types::CallerContext,
    input: Option<Value>,
    executor: Option<ApiExecutorRef>,
) -> CellValue {
    let started_at = now_millis();
    let endpoint = match &cell.def.endpoint {
        Some(e) => e.clone(),
        None => return CellValue::err("api cell has no endpoint"),
    };

    // Model/MCP pseudo-endpoints: we don't have a real provider registry
    // in the MVP, so we return a synthetic result. Callers wire up
    // real providers by replacing this evaluator at the engine level.
    if let Some(stripped) = endpoint.strip_prefix("model:") {
        return CellValue {
            data: serde_json::json!({
                "model": stripped,
                "note": "model calls not yet implemented",
            }),
            status: CellStatus::Ready,
            computed_at: Some(now_millis()),
            error: None,
            effects: vec![
                Effect::Model {
                    provider: stripped.to_string(),
                    tokens_in: None,
                    tokens_out: None,
                },
                Effect::Compute {
                    ms: now_millis().saturating_sub(started_at),
                },
            ],
        };
    }
    if endpoint.starts_with("mcp://") {
        return CellValue {
            data: serde_json::json!({
                "tool": endpoint,
                "note": "MCP tool calls not yet implemented",
            }),
            status: CellStatus::Ready,
            computed_at: Some(now_millis()),
            error: None,
            effects: vec![
                Effect::Network {
                    url: endpoint.clone(),
                    method: "MCP".to_string(),
                },
                Effect::Compute {
                    ms: now_millis().saturating_sub(started_at),
                },
            ],
        };
    }

    // Real HTTP call.
    let url = substitute(&endpoint, &ctx);
    let method = cell.def.method.clone().unwrap_or_else(|| "GET".to_string());
    let mut headers = cell.def.headers.clone();
    if method != "GET" && method != "HEAD" && !headers.contains_key("content-type") {
        headers.insert("content-type".to_string(), "application/json".to_string());
    }
    let body_str = match input {
        Some(Value::String(s)) => Some(s.clone()),
        Some(other) => Some(other.to_string()),
        None => None,
    };

    let executor: ApiExecutorRef = executor.unwrap_or_else(|| std::sync::Arc::new(ReqwestExecutor::new()));

    let result = executor.execute(&method, &url, &headers, body_str.as_deref()).await;
    let duration = now_millis().saturating_sub(started_at);

    match result {
        Ok(resp) if (200..300).contains(&resp.status) => CellValue {
            data: resp.body,
            status: CellStatus::Ready,
            computed_at: Some(now_millis()),
            error: None,
            effects: vec![
                Effect::Network {
                    url,
                    method,
                },
                Effect::Compute { ms: duration },
            ],
        },
        Ok(resp) => CellValue::err(format!("HTTP {} {}", resp.status, resp.status_text)),
        Err(err) => CellValue::err(format!("{err}")),
    }
}

/// Substitute `{{path}}` placeholders by walking into the context.
fn substitute(template: &str, ctx: &crate::types::CallerContext) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            // Find the closing `}}`
            let mut j = i + 2;
            while j + 1 < bytes.len() && !(bytes[j] == b'}' && bytes[j + 1] == b'}') {
                j += 1;
            }
            if j + 1 < bytes.len() {
                let path = &template[i + 2..j];
                out.push_str(&lookup(ctx, path));
                i = j + 2;
                continue;
            }
        }
        out.push(template[i..].chars().next().unwrap());
        i += template[i..].chars().next().unwrap().len_utf8();
    }
    out
}

fn lookup(ctx: &crate::types::CallerContext, path: &str) -> String {
    let mut cur: Option<&Value> = None;
    // Start from the context itself.
    let sheet = ctx.sheet.clone();
    let row = ctx.row.clone();
    let column = ctx.column.clone();
    let identity = ctx.identity.clone();
    let metadata = ctx.metadata.clone();

    let mut parts = path.trim().split('.');
    let first = parts.next().unwrap_or("");
    let value: Option<Value> = match first {
        "sheet" => sheet.map(Value::String),
        "row" => row,
        "column" => column,
        "identity" => identity.as_ref().map(|i| {
            serde_json::json!({
                "id": i.id,
                "type": i.kind.as_str(),
                "tags": i.tags,
            })
        }),
        "metadata" => Some(Value::Object(
            metadata
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        )),
        "caller" => ctx.caller.clone().map(Value::String),
        _ => None,
    };
    cur = value.as_ref();

    for p in parts {
        if let Some(v) = cur {
            if let Value::Object(map) = v {
                cur = map.get(p);
            } else {
                cur = None;
                break;
            }
        } else {
            break;
        }
    }

    match cur {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CellDef, CellKind};
    use std::sync::Arc;

    fn api_cell(endpoint: &str) -> Cell {
        Cell::new(CellDef {
            id: "api".into(),
            kind: CellKind::Api,
            endpoint: Some(endpoint.to_string()),
            ..Default::default()
        })
    }

    #[tokio::test]
    async fn stub_returns_canned_response() {
        let stub = StubExecutor {
            response: ApiResponse {
                status: 200,
                status_text: "OK".into(),
                headers: Default::default(),
                body: serde_json::json!({"ok": true}),
            },
        };
        let cell = api_cell("https://example.com/test");
        let v = evaluate_api(
            cell,
            crate::types::CallerContext::default(),
            None,
            Some(Arc::new(stub)),
        )
        .await;
        assert_eq!(v.status, CellStatus::Ready);
        assert_eq!(v.data, serde_json::json!({"ok": true}));
    }

    #[tokio::test]
    async fn model_pseudo_endpoint_returns_synthetic() {
        let cell = api_cell("model:gpt-4o");
        let v = evaluate_api(
            cell,
            crate::types::CallerContext::default(),
            None,
            None,
        )
        .await;
        assert_eq!(v.status, CellStatus::Ready);
        assert_eq!(v.data["model"], "gpt-4o");
    }

    #[tokio::test]
    async fn missing_endpoint_errors() {
        let cell = Cell::new(CellDef {
            id: "api".into(),
            kind: CellKind::Api,
            endpoint: None,
            ..Default::default()
        });
        let v = evaluate_api(
            cell,
            crate::types::CallerContext::default(),
            None,
            None,
        )
        .await;
        assert_eq!(v.status, CellStatus::Error);
    }

    #[test]
    fn substitute_replaces_paths() {
        let mut ctx = crate::types::CallerContext::default();
        ctx.row = Some(serde_json::json!(7));
        let s = substitute("https://example.com/r/{{row}}", &ctx);
        assert_eq!(s, "https://example.com/r/7");
    }
}
