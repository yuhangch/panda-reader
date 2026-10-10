use panda_translate::{TranslateRequest, TranslatorConfig, build};
use std::{env, fs};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let config = TranslatorConfig::load(std::path::Path::new(&args[0]))?;
    let translator = build(&config)?;
    let html = fs::read_to_string(&args[1])?;
    let started = std::time::Instant::now();
    let result = translator
        .translate(TranslateRequest {
            html,
            title: Some(args.get(3).cloned().unwrap_or_else(|| {
                "In \"Musk,\" Alex Gibney Punctures Elon's Self-Mythology".into()
            })),
            target_lang: "zh-Hans".into(),
        })
        .await?;
    let translated_chars = result.html.chars().count();
    let has_chinese = result
        .html
        .chars()
        .any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch));
    fs::write(&args[2], &result.html)?;
    println!(
        "OK output_chars={translated_chars} has_chinese={has_chinese} elapsed_seconds={:.2} requests={} input_tokens={:?} output_tokens={:?}",
        started.elapsed().as_secs_f64(),
        result.metrics.requests,
        result.metrics.input_tokens,
        result.metrics.output_tokens
    );
    println!(
        "{}",
        result
            .html
            .chars()
            .take(300)
            .collect::<String>()
            .replace('\n', " ")
    );
    if !has_chinese || translated_chars == 0 {
        anyhow::bail!("translation output did not contain Chinese text");
    }
    Ok(())
}
