//! Versioned JSON host ABI and bounded Wasmi execution.

use crate::{ArticleDocument, PluginCapability, PluginStage};
use anyhow::{Context as _, bail};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard, OnceLock};
use url::Url;
use wasmi::{
    Caller, Config, Engine, Extern, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder,
};

const MAX_MODULE_BYTES: usize = 4 * 1024 * 1024;
const MAX_GUEST_MEMORY: usize = 16 * 1024 * 1024;
const MAX_HOST_VALUE: usize = 2 * 1024 * 1024;
const MAX_HOST_RESULT_BYTES: usize = 16 * 1024 * 1024;
const MAX_HOST_READ_BYTES: usize = 16 * 1024 * 1024;
const MAX_PLUGIN_HTTP_BYTES: usize = 1024 * 1024;
const MAX_PLUGIN_HTTP_REQUESTS: u8 = 4;
const MAX_HOST_CALLS: u32 = 10_000;
const FUEL_PER_STAGE: u64 = 10_000_000;
const MODULE_CACHE_CAPACITY: usize = 16;

type CachedModule = (String, Engine, Module);
static MODULE_CACHE: OnceLock<Mutex<VecDeque<CachedModule>>> = OnceLock::new();

pub(super) struct HostState {
    document: ArticleDocument,
    title: String,
    url: String,
    stage: PluginStage,
    capabilities: Vec<PluginCapability>,
    network_hosts: Vec<String>,
    results: HashMap<i32, Vec<u8>>,
    result_bytes: usize,
    read_bytes: usize,
    next_result: i32,
    calls: u32,
    body: Option<String>,
    resolved_url: Option<String>,
    network_requests: u8,
    limits: StoreLimits,
}

pub(super) fn process(
    module_bytes: &[u8],
    module_hash: &str,
    document: ArticleDocument,
    title: &str,
    url: &str,
    stage: PluginStage,
    capabilities: &[PluginCapability],
) -> anyhow::Result<Option<(String, bool, ArticleDocument)>> {
    if module_bytes.len() > MAX_MODULE_BYTES {
        bail!("Wasm plugin exceeds the 4 MiB module limit");
    }
    let (engine, module) = cached_module(module_hash, module_bytes)?;
    let limits = StoreLimitsBuilder::new()
        .memory_size(MAX_GUEST_MEMORY)
        .memories(1)
        .tables(1)
        .table_elements(100_000)
        .build();
    let state = HostState {
        document,
        title: title.to_owned(),
        url: url.to_owned(),
        stage,
        capabilities: capabilities.to_vec(),
        network_hosts: Vec::new(),
        results: HashMap::new(),
        result_bytes: 0,
        read_bytes: 0,
        next_result: 1,
        calls: 0,
        body: None,
        resolved_url: None,
        network_requests: 0,
        limits,
    };
    let mut store = Store::new(&engine, state);
    store.limiter(|state| &mut state.limits);
    store.set_fuel(FUEL_PER_STAGE)?;
    let mut linker = Linker::new(&engine);
    linker
        .func_wrap("panda_v1", "host_call", host_call)
        .context("define host_call import")?;
    linker
        .func_wrap("panda_v1", "result_len", result_len)
        .context("define result_len import")?;
    linker
        .func_wrap("panda_v1", "result_read", result_read)
        .context("define result_read import")?;
    linker
        .func_wrap("panda_v1", "result_drop", result_drop)
        .context("define result_drop import")?;
    let instance = linker
        .instantiate_and_start(&mut store, &module)
        .context("instantiate Wasm plugin")?;
    let entry = instance
        .get_typed_func::<i32, i32>(&store, "panda_process")
        .context("plugin must export panda_process(i32) -> i32")?;
    let result = entry
        .call(&mut store, stage as i32)
        .context("Wasm plugin execution failed")?;
    let state = store.into_data();
    match result {
        0 => Ok(None),
        1 => {
            let body_selected = state.body.is_some();
            let body = state.body.unwrap_or_else(|| state.document.serialize());
            if body.trim().is_empty() {
                bail!("Wasm plugin produced empty article content");
            }
            Ok(Some((body, body_selected, state.document)))
        }
        other => bail!("Wasm plugin returned failure code {other}"),
    }
}

pub(super) fn resolve_url(
    module_bytes: &[u8],
    module_hash: &str,
    source_url: &str,
    capabilities: &[PluginCapability],
    network_hosts: &[String],
) -> anyhow::Result<Option<String>> {
    if module_bytes.len() > MAX_MODULE_BYTES {
        bail!("Wasm plugin exceeds the 4 MiB module limit");
    }
    let module_bytes = module_bytes.to_vec();
    let module_hash = module_hash.to_owned();
    let source_url = source_url.to_owned();
    let capabilities = capabilities.to_vec();
    let network_hosts = network_hosts.to_vec();
    // Network host calls use reqwest's blocking client. Isolate Wasmi from the
    // async extraction runtime so blocking HTTP never stalls that runtime.
    std::thread::Builder::new()
        .name("panda-plugin-url-resolver".into())
        .spawn(move || {
            resolve_url_blocking(
                &module_bytes,
                &module_hash,
                &source_url,
                capabilities,
                network_hosts,
            )
        })?
        .join()
        .map_err(|_| anyhow::anyhow!("Wasm URL resolver thread panicked"))?
}

fn resolve_url_blocking(
    module_bytes: &[u8],
    module_hash: &str,
    source_url: &str,
    capabilities: Vec<PluginCapability>,
    network_hosts: Vec<String>,
) -> anyhow::Result<Option<String>> {
    let (engine, module) = cached_module(module_hash, module_bytes)?;
    let document = ArticleDocument::parse("")?;
    let limits = StoreLimitsBuilder::new()
        .memory_size(MAX_GUEST_MEMORY)
        .memories(1)
        .tables(1)
        .table_elements(100_000)
        .build();
    let state = HostState {
        document,
        title: String::new(),
        url: source_url.to_owned(),
        stage: PluginStage::ResolveUrl,
        capabilities,
        network_hosts,
        results: HashMap::new(),
        result_bytes: 0,
        read_bytes: 0,
        next_result: 1,
        calls: 0,
        body: None,
        resolved_url: None,
        network_requests: 0,
        limits,
    };
    let mut store = Store::new(&engine, state);
    store.limiter(|state| &mut state.limits);
    store.set_fuel(FUEL_PER_STAGE)?;
    let mut linker = Linker::new(&engine);
    linker.func_wrap("panda_v1", "host_call", host_call)?;
    linker.func_wrap("panda_v1", "result_len", result_len)?;
    linker.func_wrap("panda_v1", "result_read", result_read)?;
    linker.func_wrap("panda_v1", "result_drop", result_drop)?;
    let instance = linker
        .instantiate_and_start(&mut store, &module)
        .context("instantiate Wasm URL resolver")?;
    let entry = instance
        .get_typed_func::<i32, i32>(&store, "panda_process")
        .context("plugin must export panda_process(i32) -> i32")?;
    let result = entry
        .call(&mut store, PluginStage::ResolveUrl as i32)
        .context("Wasm URL resolver execution failed")?;
    let state = store.into_data();
    match result {
        0 => Ok(None),
        1 => {
            let Some(url) = state.resolved_url else {
                bail!("URL resolver returned success without setting a URL");
            };
            validate_resolved_url(source_url, &url)?;
            Ok((url != source_url).then_some(url))
        }
        other => bail!("Wasm URL resolver returned failure code {other}"),
    }
}

fn validate_resolved_url(source: &str, resolved: &str) -> anyhow::Result<()> {
    let source = Url::parse(source)?;
    let resolved = Url::parse(resolved)?;
    if !matches!(resolved.scheme(), "http" | "https")
        || !resolved.username().is_empty()
        || resolved.password().is_some()
        || resolved.host_str().is_none()
    {
        bail!("URL resolver returned an unsafe destination URL");
    }
    if resolved.host_str() == source.host_str() && resolved == source {
        bail!("URL resolver did not change the URL");
    }
    Ok(())
}

fn cached_module(hash: &str, bytes: &[u8]) -> anyhow::Result<(Engine, Module)> {
    let cache = MODULE_CACHE.get_or_init(|| Mutex::new(VecDeque::new()));
    {
        let mut entries = lock_module_cache(cache);
        if let Some(index) = entries.iter().position(|(cached, _, _)| cached == hash) {
            let (_, engine, module) = entries.remove(index).expect("module index was found");
            entries.push_front((hash.to_owned(), engine.clone(), module.clone()));
            return Ok((engine, module));
        }
    }
    let mut config = Config::default();
    config.consume_fuel(true);
    let engine = Engine::new(&config);
    let module = Module::new(&engine, bytes).context("invalid Wasm plugin module")?;
    let mut entries = lock_module_cache(cache);
    entries.retain(|(cached, _, _)| cached != hash);
    entries.push_front((hash.to_owned(), engine.clone(), module.clone()));
    entries.truncate(MODULE_CACHE_CAPACITY);
    Ok((engine, module))
}

fn lock_module_cache(
    cache: &Mutex<VecDeque<CachedModule>>,
) -> MutexGuard<'_, VecDeque<CachedModule>> {
    match cache.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            guard.clear();
            cache.clear_poison();
            eprintln!("Wasm module cache recovered after a panic");
            guard
        }
    }
}

fn host_call(
    mut caller: Caller<'_, HostState>,
    pointer: i32,
    length: i32,
) -> Result<i32, wasmi::Error> {
    let request = read_guest(&caller, pointer, length)?;
    let request: Value = serde_json::from_slice(&request)
        .map_err(|error| wasmi::Error::new(format!("invalid host request JSON: {error}")))?;
    {
        let state = caller.data_mut();
        state.calls = state.calls.saturating_add(1);
        if state.calls > MAX_HOST_CALLS {
            return Err(wasmi::Error::new("host call budget exceeded"));
        }
    }
    let result = dispatch_host_call(caller.data_mut(), request)?;
    let bytes = serde_json::to_vec(&result)
        .map_err(|error| wasmi::Error::new(format!("serialize host result: {error}")))?;
    if bytes.len() > MAX_HOST_VALUE {
        return Err(wasmi::Error::new("host response exceeds the 2 MiB limit"));
    }
    if caller.data().result_bytes.saturating_add(bytes.len()) > MAX_HOST_RESULT_BYTES {
        return Err(wasmi::Error::new(
            "plugin exceeded the total host result memory limit",
        ));
    }
    let state = caller.data_mut();
    let handle = state.next_result;
    state.next_result = state.next_result.saturating_add(1);
    state.result_bytes += bytes.len();
    state.results.insert(handle, bytes);
    Ok(handle)
}

fn result_len(caller: Caller<'_, HostState>, handle: i32) -> i32 {
    caller
        .data()
        .results
        .get(&handle)
        .and_then(|bytes| i32::try_from(bytes.len()).ok())
        .unwrap_or(-1)
}

fn result_read(
    mut caller: Caller<'_, HostState>,
    handle: i32,
    pointer: i32,
    capacity: i32,
) -> Result<i32, wasmi::Error> {
    let bytes = caller
        .data()
        .results
        .get(&handle)
        .cloned()
        .ok_or_else(|| wasmi::Error::new("unknown result handle"))?;
    if caller.data().read_bytes.saturating_add(bytes.len()) > MAX_HOST_READ_BYTES {
        return Err(wasmi::Error::new(
            "plugin exceeded the total host copy limit",
        ));
    }
    let capacity = usize::try_from(capacity).map_err(|_| wasmi::Error::new("invalid capacity"))?;
    if capacity < bytes.len() {
        return Err(wasmi::Error::new("result buffer is too small"));
    }
    let memory = guest_memory(&caller)?;
    memory
        .write(
            &mut caller,
            usize::try_from(pointer).map_err(|_| wasmi::Error::new("invalid pointer"))?,
            &bytes,
        )
        .map_err(|error| wasmi::Error::new(format!("invalid guest memory range: {error}")))?;
    caller.data_mut().read_bytes += bytes.len();
    i32::try_from(bytes.len()).map_err(|_| wasmi::Error::new("result too large"))
}

fn result_drop(mut caller: Caller<'_, HostState>, handle: i32) -> i32 {
    let state = caller.data_mut();
    if let Some(bytes) = state.results.remove(&handle) {
        state.result_bytes = state.result_bytes.saturating_sub(bytes.len());
        1
    } else {
        0
    }
}

fn dispatch_host_call(state: &mut HostState, request: Value) -> Result<Value, wasmi::Error> {
    let op = request
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| wasmi::Error::new("host request is missing op"))?;
    match op {
        "context" => {
            if !state.capabilities.contains(&PluginCapability::DocumentRead)
                && !state
                    .capabilities
                    .contains(&PluginCapability::NetworkRequest)
            {
                return Err(wasmi::Error::new(
                    "plugin must declare document.read or network_request to read context",
                ));
            }
            Ok(json!({
                "title": state.title,
                "url": state.url,
                "stage": match state.stage { PluginStage::Prepare => "prepare", PluginStage::Cleanup => "cleanup", PluginStage::ResolveUrl => "resolve_url" },
            }))
        }
        "query" => {
            require(state, PluginCapability::DocumentRead)?;
            let selector = required_string(&request, "selector")?;
            let handles = state.document.query(selector).map_err(host_error)?;
            Ok(json!({ "nodes": handles }))
        }
        "text" | "html" | "attr" => {
            require(state, PluginCapability::DocumentRead)?;
            let handle = required_handle(&request)?;
            let value = match op {
                "text" => Value::String(state.document.text(handle).map_err(host_error)?),
                "html" => Value::String(state.document.html(handle).map_err(host_error)?),
                _ => state
                    .document
                    .attribute(handle, required_string(&request, "name")?)
                    .map_err(host_error)?
                    .map(Value::String)
                    .unwrap_or(Value::Null),
            };
            Ok(json!({ "value": value }))
        }
        "remove" => {
            require(state, PluginCapability::DocumentWrite)?;
            state
                .document
                .remove(required_handle(&request)?)
                .map_err(host_error)?;
            Ok(json!({ "removed": true }))
        }
        "set_attr" => {
            require(state, PluginCapability::DocumentWrite)?;
            let name = required_string(&request, "name")?;
            let value = request.get("value").and_then(Value::as_str);
            state
                .document
                .set_attribute(required_handle(&request)?, name, value)
                .map_err(host_error)?;
            Ok(json!({ "updated": true }))
        }
        "set_body" => {
            require(state, PluginCapability::ArticleBodyWrite)?;
            let body = required_string(&request, "html")?;
            state.body = Some(body.to_owned());
            Ok(json!({ "updated": true }))
        }
        "replace_html" => {
            require(state, PluginCapability::DocumentWrite)?;
            let body = required_string(&request, "html")?;
            state
                .document
                .set_body_html(required_handle(&request)?, body)
                .map_err(host_error)?;
            Ok(json!({ "updated": true }))
        }
        "resolve_url" => {
            require(state, PluginCapability::DocumentRead)?;
            let base = Url::parse(&state.url).map_err(host_error)?;
            let resolved = base
                .join(required_string(&request, "url")?)
                .map_err(host_error)?;
            Ok(json!({ "url": resolved.as_str() }))
        }
        "http_request" => plugin_http_request(state, &request),
        "set_resolved_url" => {
            require(state, PluginCapability::ArticleUrlWrite)?;
            if state.stage != PluginStage::ResolveUrl {
                return Err(wasmi::Error::new(
                    "set_resolved_url is only available during resolve_url",
                ));
            }
            state.resolved_url = Some(required_string(&request, "url")?.to_owned());
            Ok(json!({ "updated": true }))
        }
        "log" => {
            let message = required_string(&request, "value")?;
            if message.len() > 4096 {
                return Err(wasmi::Error::new("plugin log message exceeds 4 KiB"));
            }
            eprintln!("article plugin: {message}");
            Ok(json!({ "logged": true }))
        }
        _ => Err(wasmi::Error::new(format!(
            "unsupported host operation `{op}`"
        ))),
    }
}

fn plugin_http_request(state: &mut HostState, request: &Value) -> Result<Value, wasmi::Error> {
    require(state, PluginCapability::NetworkRequest)?;
    if state.stage != PluginStage::ResolveUrl {
        return Err(wasmi::Error::new(
            "network requests are only available during resolve_url",
        ));
    }
    state.network_requests = state.network_requests.saturating_add(1);
    if state.network_requests > MAX_PLUGIN_HTTP_REQUESTS {
        return Err(wasmi::Error::new("plugin HTTP request budget exceeded"));
    }
    let url = Url::parse(required_string(request, "url")?)
        .map_err(|error| wasmi::Error::new(format!("invalid plugin request URL: {error}")))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|port| port != 443)
        || !state
            .network_hosts
            .iter()
            .any(|host| network_host_matches(host, url.host_str().unwrap_or_default()))
    {
        return Err(wasmi::Error::new(
            "plugin request URL is outside its HTTPS host allowlist",
        ));
    }
    let method = required_string(request, "method")?;
    let method = match method {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        _ => return Err(wasmi::Error::new("plugin HTTP method must be GET or POST")),
    };
    if let Some(headers) = request.get("headers").and_then(Value::as_object)
        && headers.len() > 32
    {
        return Err(wasmi::Error::new(
            "plugin HTTP request has too many headers",
        ));
    }
    let body = request
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if body.len() > MAX_PLUGIN_HTTP_BYTES {
        return Err(wasmi::Error::new("plugin HTTP request body is too large"));
    }
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("PandaReader/0.2")
        .build()
        .map_err(host_error)?;
    let mut outgoing = client.request(method, url.clone());
    if let Some(headers) = request.get("headers").and_then(Value::as_object) {
        for (name, value) in headers {
            let value = value
                .as_str()
                .ok_or_else(|| wasmi::Error::new("plugin HTTP header values must be strings"))?;
            let name =
                reqwest::header::HeaderName::from_bytes(name.as_bytes()).map_err(host_error)?;
            let value = reqwest::header::HeaderValue::from_str(value).map_err(host_error)?;
            if matches!(
                name,
                reqwest::header::HOST | reqwest::header::CONTENT_LENGTH
            ) {
                return Err(wasmi::Error::new(
                    "plugin cannot override Host or Content-Length",
                ));
            }
            outgoing = outgoing.header(name, value);
        }
    }
    if !body.is_empty() {
        outgoing = outgoing.body(body.to_owned());
    }
    let mut response = outgoing.send().map_err(host_error)?;
    let status = response.status().as_u16();
    let final_url = response.url().to_string();
    let mut bytes = Vec::new();
    use std::io::Read as _;
    response
        .by_ref()
        .take((MAX_PLUGIN_HTTP_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(host_error)?;
    if bytes.len() > MAX_PLUGIN_HTTP_BYTES {
        return Err(wasmi::Error::new("plugin HTTP response exceeds 1 MiB"));
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Ok(json!({ "status": status, "url": final_url, "body": text }))
}

fn network_host_matches(pattern: &str, host: &str) -> bool {
    match pattern.strip_prefix("*.") {
        Some(domain) => {
            host.eq_ignore_ascii_case(domain)
                || host
                    .strip_suffix(domain)
                    .is_some_and(|prefix| prefix.ends_with('.'))
        }
        None => host.eq_ignore_ascii_case(pattern),
    }
}

fn require(state: &HostState, capability: PluginCapability) -> Result<(), wasmi::Error> {
    if state.capabilities.contains(&capability) {
        Ok(())
    } else {
        Err(wasmi::Error::new(
            "plugin did not declare the required capability",
        ))
    }
}

fn required_string<'a>(request: &'a Value, field: &str) -> Result<&'a str, wasmi::Error> {
    request
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| wasmi::Error::new(format!("host request requires string `{field}`")))
}

fn required_handle(request: &Value) -> Result<u32, wasmi::Error> {
    request
        .get("node")
        .and_then(Value::as_u64)
        .and_then(|handle| u32::try_from(handle).ok())
        .ok_or_else(|| wasmi::Error::new("host request requires a valid node handle"))
}

fn read_guest(
    caller: &Caller<'_, HostState>,
    pointer: i32,
    length: i32,
) -> Result<Vec<u8>, wasmi::Error> {
    let length = usize::try_from(length).map_err(|_| wasmi::Error::new("invalid input length"))?;
    if length > MAX_HOST_VALUE {
        return Err(wasmi::Error::new("host request exceeds the 2 MiB limit"));
    }
    let mut bytes = vec![0; length];
    guest_memory(caller)?
        .read(
            caller,
            usize::try_from(pointer).map_err(|_| wasmi::Error::new("invalid input pointer"))?,
            &mut bytes,
        )
        .map_err(|error| wasmi::Error::new(format!("invalid guest memory range: {error}")))?;
    Ok(bytes)
}

fn guest_memory(caller: &Caller<'_, HostState>) -> Result<Memory, wasmi::Error> {
    caller
        .get_export("memory")
        .and_then(Extern::into_memory)
        .ok_or_else(|| wasmi::Error::new("plugin must export linear memory as `memory`"))
}

fn host_error(error: impl std::fmt::Display) -> wasmi::Error {
    wasmi::Error::new(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // A small wasm32 module exporting panda_process(stage) -> APPLIED.
    fn applied_module() -> Vec<u8> {
        let mut bytes = b"\0asm\x01\0\0\0".to_vec();
        bytes.extend_from_slice(&[
            1, 6, 1, 0x60, 1, 0x7f, 1, 0x7f, // (i32) -> i32
            3, 2, 1, 0, // one function, type 0
            7, 17, 1, 13, b'p', b'a', b'n', b'd', b'a', b'_', b'p', b'r', b'o', b'c', b'e', b's',
            b's', 0, 0, 10, 9, 1, 7, 0, 0x20, 0, 0x1a, 0x41, 1, 0x0b,
        ]);
        bytes
    }

    fn looping_module() -> Vec<u8> {
        let mut bytes = b"\0asm\x01\0\0\0".to_vec();
        bytes.extend_from_slice(&[
            1, 6, 1, 0x60, 1, 0x7f, 1, 0x7f, // (i32) -> i32
            3, 2, 1, 0, // one function, type 0
            7, 17, 1, 13, b'p', b'a', b'n', b'd', b'a', b'_', b'p', b'r', b'o', b'c', b'e', b's',
            b's', 0, 0, 10, 11, 1, 9, 0, 0x03, 0x40, 0x0c, 0, 0x0b, 0x41, 0, 0x0b,
        ]);
        bytes
    }

    #[test]
    fn runs_a_valid_module_with_the_expected_stage_abi() {
        let output = process(
            &applied_module(),
            "test-module",
            ArticleDocument::parse("<p>article body</p>").unwrap(),
            "Title",
            "https://example.com/story",
            PluginStage::Cleanup,
            &[],
        )
        .unwrap();
        let (html, selected_body, _) = output.unwrap();
        assert!(html.contains("article body"));
        assert!(!selected_body);
    }

    #[test]
    fn rejects_invalid_module_bytes() {
        assert!(
            process(
                b"not wasm",
                "invalid-module",
                ArticleDocument::parse("<p>body</p>").unwrap(),
                "Title",
                "https://example.com/story",
                PluginStage::Cleanup,
                &[],
            )
            .is_err()
        );
    }

    #[test]
    fn instruction_fuel_stops_an_infinite_plugin_loop() {
        let error = process(
            &looping_module(),
            "looping-module",
            ArticleDocument::parse("<p>body</p>").unwrap(),
            "Title",
            "https://example.com/story",
            PluginStage::Cleanup,
            &[],
        )
        .err()
        .unwrap();
        assert!(format!("{error:#}").contains("fuel"));
    }

    #[test]
    fn plugin_http_requests_require_https_and_declared_hosts() {
        let limits = StoreLimitsBuilder::new().build();
        let mut state = HostState {
            document: ArticleDocument::parse("<html></html>").unwrap(),
            title: String::new(),
            url: "https://news.google.com/rss/articles/id".into(),
            stage: PluginStage::ResolveUrl,
            capabilities: vec![PluginCapability::NetworkRequest],
            network_hosts: vec!["news.google.com".into()],
            results: HashMap::new(),
            result_bytes: 0,
            read_bytes: 0,
            next_result: 1,
            calls: 0,
            body: None,
            resolved_url: None,
            network_requests: 0,
            limits,
        };
        let unlisted = plugin_http_request(
            &mut state,
            &json!({"method":"GET","url":"https://example.com/"}),
        );
        assert!(unlisted.is_err());
        assert_eq!(state.network_requests, 1);

        let insecure = plugin_http_request(
            &mut state,
            &json!({"method":"GET","url":"http://news.google.com/"}),
        );
        assert!(insecure.is_err());
        assert_eq!(state.network_requests, 2);
    }

    #[test]
    fn resolver_destination_rejects_credentials_and_non_web_schemes() {
        assert!(
            validate_resolved_url(
                "https://news.google.com/article",
                "https://user:pass@example.com/story"
            )
            .is_err()
        );
        assert!(
            validate_resolved_url("https://news.google.com/article", "file:///etc/passwd").is_err()
        );
        assert!(
            validate_resolved_url(
                "https://news.google.com/article",
                "https://example.com/story"
            )
            .is_ok()
        );
    }

    #[test]
    fn resolver_stage_runs_the_wasm_abi_without_network_access() {
        let error = resolve_url(
            &applied_module(),
            "resolver-test-module",
            "https://news.google.com/rss/articles/example",
            &[],
            &[],
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("without setting a URL"));
    }

    #[test]
    fn bundled_google_resolver_wasm_starts_without_implicit_network_permission() {
        let module =
            include_bytes!("../../../plugins/community/community.google-news-resolver/plugin.wasm");
        let error = resolve_url(
            module,
            "google-resolver-fixture",
            "https://news.google.com/rss/articles/example",
            &[],
            &["news.google.com".into()],
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("declare document.read or network_request"));
    }
}
