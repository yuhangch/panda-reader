use panda_plugin_sdk::Stage;
use serde_json::Value;

fn process(stage: Stage) -> Result<bool, String> {
    if stage != Stage::Prepare {
        return Ok(false);
    }
    let Some(script) = panda_plugin_sdk::query("script#article-data")?.into_iter().next() else {
        return Ok(false);
    };
    let json: Value = serde_json::from_str(&panda_plugin_sdk::text(script)?)
        .map_err(|error| format!("article JSON is invalid: {error}"))?;
    let Some(body) = json.get("articleBody").and_then(Value::as_str) else {
        return Ok(false);
    };
    if body.trim().is_empty() {
        return Ok(false);
    }
    panda_plugin_sdk::set_body(body)?;
    Ok(true)
}

panda_plugin_sdk::export_plugin!(process);
