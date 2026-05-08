use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use deno_core::OpState;
use deno_core::op2;
use deno_error::JsErrorBox;
use serde::{Deserialize, Serialize};

use crate::executors::deno::permissions::PermissionChecker;

/// Options for the fetch call (method, headers, body).
#[derive(Deserialize, Default)]
pub struct FetchOptions {
    /// Method.
    pub method: Option<String>,
    /// Headers.
    pub headers: Option<HashMap<String, String>>,
    /// Body.
    pub body: Option<String>,
}

/// Response returned to JavaScript from fetch.
#[derive(Serialize)]
pub struct FetchResponse {
    /// Status.
    pub status: u16,
    /// Ok.
    pub ok: bool,
    /// Headers.
    pub headers: HashMap<String, String>,
    /// Body.
    pub body: String,
}

#[op2]
#[serde]
/// Op fetch.
pub async fn op_fetch(
    state: Rc<RefCell<OpState>>,
    #[string] url: String,
    #[serde] options: Option<FetchOptions>,
) -> Result<FetchResponse, JsErrorBox> {
    // 1. Parse URL to extract host for permission check.
    let parsed = url::Url::parse(&url)
        .map_err(|e| JsErrorBox::generic(format!("Invalid URL '{url}': {e}")))?;

    let host = parsed
        .host_str()
        .ok_or_else(|| JsErrorBox::generic(format!("URL '{url}' has no host")))?
        .to_string();

    // 2. Check net permission.
    {
        let state = state.borrow();
        let checker = state.borrow::<PermissionChecker>();
        checker
            .check_net(&host)
            .map_err(|e| JsErrorBox::generic(e.to_string()))?;
    }

    // 3. Build the request.
    let opts = options.unwrap_or_default();
    let method = opts.method.as_deref().unwrap_or("GET");

    let client = reqwest::Client::new();
    let mut builder = match method.to_uppercase().as_str() {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        "PUT" => client.put(&url),
        "DELETE" => client.delete(&url),
        "PATCH" => client.patch(&url),
        "HEAD" => client.head(&url),
        other => {
            return Err(JsErrorBox::generic(format!(
                "Unsupported HTTP method: {other}"
            )));
        }
    };

    if let Some(ref headers) = opts.headers {
        for (key, value) in headers {
            builder = builder.header(key.as_str(), value.as_str());
        }
    }

    if let Some(ref body) = opts.body {
        builder = builder.body(body.clone());
    }

    // 4. Execute request.
    let response = builder
        .send()
        .await
        .map_err(|e| JsErrorBox::generic(format!("Fetch failed for '{url}': {e}")))?;

    // 5. Build response.
    let status = response.status().as_u16();
    let ok = response.status().is_success();
    let headers: HashMap<String, String> = response
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let body = response
        .text()
        .await
        .map_err(|e| JsErrorBox::generic(format!("Failed to read response body: {e}")))?;

    Ok(FetchResponse {
        status,
        ok,
        headers,
        body,
    })
}
