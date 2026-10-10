//! Rust SDK for Panda Reader's versioned WebAssembly plugin ABI.

pub const ABI_VERSION: u32 = 1;

/// Result codes returned by the `panda_process` export.
pub mod result {
    pub const NOT_HANDLED: i32 = 0;
    pub const APPLIED: i32 = 1;
    pub const FAILED: i32 = -1;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Stage {
    Prepare = 0,
    Cleanup = 1,
    ResolveUrl = 2,
}

impl TryFrom<i32> for Stage {
    type Error = String;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Prepare),
            1 => Ok(Self::Cleanup),
            2 => Ok(Self::ResolveUrl),
            _ => Err(format!("unknown Panda Reader plugin stage: {value}")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Node(pub u32);

#[derive(Clone, Debug, serde::Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    pub url: String,
    pub body: String,
}

#[cfg(target_arch = "wasm32")]
mod host {
    use super::{HttpResponse, Node};
    use serde::{Deserialize, Serialize};

    #[link(wasm_import_module = "panda_v1")]
    unsafe extern "C" {
        fn host_call(pointer: i32, length: i32) -> i32;
        fn result_len(handle: i32) -> i32;
        fn result_read(handle: i32, pointer: i32, capacity: i32) -> i32;
        fn result_drop(handle: i32) -> i32;
    }

    #[derive(Deserialize)]
    struct QueryResult {
        nodes: Vec<Node>,
    }

    #[derive(Deserialize)]
    struct ValueResult {
        value: serde_json::Value,
    }

    #[derive(Serialize)]
    struct Request<'a> {
        op: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        selector: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        node: Option<Node>,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        html: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        url: Option<&'a str>,
    }

    fn request(request: &Request<'_>) -> Result<serde_json::Value, String> {
        let input = serde_json::to_vec(request).map_err(|error| error.to_string())?;
        request_bytes(&input)
    }

    fn request_bytes(input: &[u8]) -> Result<serde_json::Value, String> {
        let handle = unsafe { host_call(input.as_ptr() as i32, input.len() as i32) };
        if handle <= 0 {
            return Err(format!("Panda Reader host call failed ({handle})"));
        }
        let length = unsafe { result_len(handle) };
        if length < 0 || length as usize > 2 * 1024 * 1024 {
            unsafe {
                result_drop(handle);
            }
            return Err("Panda Reader response is invalid or too large".into());
        }
        let mut output = vec![0; length as usize];
        let copied = unsafe { result_read(handle, output.as_mut_ptr() as i32, length) };
        unsafe {
            result_drop(handle);
        }
        if copied != length {
            return Err("Panda Reader returned a truncated response".into());
        }
        serde_json::from_slice(&output).map_err(|error| error.to_string())
    }

    pub fn context() -> Result<serde_json::Value, String> {
        request(&Request {
            op: "context",
            selector: None,
            node: None,
            name: None,
            value: None,
            html: None,
            url: None,
        })
    }

    pub fn query(selector: &str) -> Result<Vec<Node>, String> {
        request(&Request {
            op: "query",
            selector: Some(selector),
            node: None,
            name: None,
            value: None,
            html: None,
            url: None,
        })
        .and_then(|value| {
            serde_json::from_value::<QueryResult>(value)
                .map(|r| r.nodes)
                .map_err(|e| e.to_string())
        })
    }

    pub fn text(node: Node) -> Result<String, String> {
        node_value("text", node, None)
    }
    pub fn html(node: Node) -> Result<String, String> {
        node_value("html", node, None)
    }
    pub fn attribute(node: Node, name: &str) -> Result<Option<String>, String> {
        let response = request(&Request {
            op: "attr",
            selector: None,
            node: Some(node),
            name: Some(name),
            value: None,
            html: None,
            url: None,
        })?;
        let value = serde_json::from_value::<ValueResult>(response)
            .map_err(|e| e.to_string())?
            .value;
        Ok(value.as_str().map(str::to_owned))
    }
    fn node_value(op: &str, node: Node, name: Option<&str>) -> Result<String, String> {
        let host_request = Request {
            op,
            selector: None,
            node: Some(node),
            name,
            value: None,
            html: None,
            url: None,
        };
        let response = request(&host_request)?;
        serde_json::from_value::<ValueResult>(response)
            .map(|r| r.value.as_str().unwrap_or_default().to_owned())
            .map_err(|e| e.to_string())
    }

    pub fn remove(node: Node) -> Result<(), String> {
        call_empty(&Request {
            op: "remove",
            selector: None,
            node: Some(node),
            name: None,
            value: None,
            html: None,
            url: None,
        })
    }
    pub fn set_attribute(node: Node, name: &str, value: &str) -> Result<(), String> {
        call_empty(&Request {
            op: "set_attr",
            selector: None,
            node: Some(node),
            name: Some(name),
            value: Some(value),
            html: None,
            url: None,
        })
    }
    pub fn remove_attribute(node: Node, name: &str) -> Result<(), String> {
        call_empty(&Request {
            op: "set_attr",
            selector: None,
            node: Some(node),
            name: Some(name),
            value: None,
            html: None,
            url: None,
        })
    }
    pub fn set_body(html: &str) -> Result<(), String> {
        call_empty(&Request {
            op: "set_body",
            selector: None,
            node: None,
            name: None,
            value: None,
            html: Some(html),
            url: None,
        })
    }
    pub fn replace_html(node: Node, html: &str) -> Result<(), String> {
        call_empty(&Request {
            op: "replace_html",
            selector: None,
            node: Some(node),
            name: None,
            value: None,
            html: Some(html),
            url: None,
        })
    }
    pub fn resolve_url(url: &str) -> Result<String, String> {
        let value = request(&Request {
            op: "resolve_url",
            selector: None,
            node: None,
            name: None,
            value: None,
            html: None,
            url: Some(url),
        })?;
        value
            .get("url")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "invalid URL response".into())
    }
    pub fn set_resolved_url(url: &str) -> Result<(), String> {
        call_empty(&Request {
            op: "set_resolved_url",
            selector: None,
            node: None,
            name: None,
            value: None,
            html: None,
            url: Some(url),
        })
    }
    pub fn http_request(
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Result<HttpResponse, String> {
        #[derive(Serialize)]
        struct NetworkRequest<'a> {
            op: &'static str,
            method: &'a str,
            url: &'a str,
            headers: std::collections::BTreeMap<&'a str, &'a str>,
            body: &'a str,
        }
        let request = NetworkRequest {
            op: "http_request",
            method,
            url,
            headers: headers.iter().copied().collect(),
            body,
        };
        let input = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
        let response: serde_json::Value = request_bytes(&input)?;
        serde_json::from_value(response).map_err(|error| error.to_string())
    }
    pub fn log(message: &str) -> Result<(), String> {
        call_empty(&Request {
            op: "log",
            selector: None,
            node: None,
            name: None,
            value: Some(message),
            html: None,
            url: None,
        })
    }
    fn call_empty(request: &Request<'_>) -> Result<(), String> {
        self::request(request).map(|_| ())
    }
}

#[cfg(target_arch = "wasm32")]
pub use host::{
    attribute, context, html, http_request, log, query, remove, remove_attribute, replace_html,
    resolve_url, set_attribute, set_body, set_resolved_url, text,
};

/// Export a Rust handler using the application ABI's standard stage/result values.
#[macro_export]
macro_rules! export_plugin {
    ($handler:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn panda_process(stage: i32) -> i32 {
            let Ok(stage) = $crate::Stage::try_from(stage) else {
                return $crate::result::FAILED;
            };
            match $handler(stage) {
                Ok(true) => $crate::result::APPLIED,
                Ok(false) => $crate::result::NOT_HANDLED,
                Err(_) => $crate::result::FAILED,
            }
        }
    };
}
