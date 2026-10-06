use panda_plugins::{PluginRegistry, PluginSettings, PluginStage};
use std::{env, fs, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("panda-plugin-runner: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<()> {
    let mut args = env::args_os().skip(1);
    let plugin_dir = PathBuf::from(args.next().ok_or_else(usage)?);
    let article_url = args
        .next()
        .ok_or_else(usage)?
        .to_string_lossy()
        .into_owned();
    let title = args
        .next()
        .ok_or_else(usage)?
        .to_string_lossy()
        .into_owned();
    let input_path = PathBuf::from(args.next().ok_or_else(usage)?);
    if args.next().is_some() {
        return Err(usage().into());
    }
    if !plugin_dir.is_dir() {
        anyhow::bail!("plugin directory does not exist: {}", plugin_dir.display());
    }
    let source = fs::read_to_string(input_path)?;
    let registry = PluginRegistry::load(&plugin_dir, &PluginSettings::default(), 1);
    for diagnostic in registry.diagnostics() {
        eprintln!("plugin {}: {}", diagnostic.plugin_id, diagnostic.message);
    }
    let prepared = registry.process(&source, &article_url, &title, PluginStage::Prepare);
    for diagnostic in prepared.diagnostics {
        eprintln!("plugin {}: {}", diagnostic.plugin_id, diagnostic.message);
    }
    let cleaned = registry.process(&prepared.html, &article_url, &title, PluginStage::Cleanup);
    for diagnostic in cleaned.diagnostics {
        eprintln!("plugin {}: {}", diagnostic.plugin_id, diagnostic.message);
    }
    print!("{}", cleaned.html);
    Ok(())
}

fn usage() -> anyhow::Error {
    anyhow::anyhow!("usage: panda-plugin-runner <plugin-dir> <article-url> <title> <fixture.html>")
}
