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
    results: HashMap<i32, Vec<u8>>,
    result_bytes: usize,
    read_bytes: usize,
    next_result: i32,
    calls: u32,
    body: Option<String>,
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
        results: HashMap::new(),
        result_bytes: 0,
        read_bytes: 0,
        next_result: 1,
        calls: 0,
        body: None,
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
            require(state, PluginCapability::DocumentRead)?;
            Ok(json!({
                "title": state.title,
                "url": state.url,
                "stage": match state.stage { PluginStage::Prepare => "prepare", PluginStage::Cleanup => "cleanup" },
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
}
