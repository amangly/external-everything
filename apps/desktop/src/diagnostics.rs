//! Diagnostics for the shipped single executable; boot verification stays read-only.
use crate::{bundle, catalog};
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};

fn removed(root: &Path) -> Result<bool> {
    Ok(!root.try_exists()?)
}

pub fn run(args: &[String]) -> Result<bool> {
    if args == ["--inspect-bundle"] {
        println!("{}", serde_json::to_string(&bundle::inspect()?)?);
        return Ok(true);
    }
    if args == ["--verify-bundle"] {
        let report = bundle::inspect()?;
        ensure!(
            report.single_file,
            "Bundle verification requires a self-contained build"
        );
        let boot = bundle::stage_boot()?.context("Embedded boot payloads are absent")?;
        let boot_root = boot.root().to_path_buf();
        let boot_identities = hyperrev_loader::files::source_identities(&boot_root)?;
        let game = bundle::stage_game("pointblank")?.context("Embedded runtime is absent")?;
        let game_root = game.root().to_path_buf();
        let executable = staged_executable(&game)?;
        let contract = external_runtime::launch::inspect_contract(&executable, "pointblank")?;
        drop(game);
        drop(boot);
        let boot_cleaned = removed(&boot_root)?;
        let game_cleaned = removed(&game_root)?;
        ensure!(
            boot_cleaned && game_cleaned,
            "Bundle verification left temporary payloads behind"
        );
        println!(
            "{}",
            serde_json::json!({"single_file":true,"assets":report.assets,"boot_payloads":boot_identities,"runtime_contract":contract,"boot_staging_cleaned":boot_cleaned,"game_staging_cleaned":game_cleaned,"boot_mutations":false})
        );
        return Ok(true);
    }
    if args.first().is_some_and(|arg| arg == "--runtime") {
        let forwarded = &args[1..];
        ensure!(
            forwarded.first().is_some_and(|arg| matches!(
                arg.as_str(),
                "contract"
                    | "--self-test"
                    | "--snapshot"
                    | "--snapshot-scan"
                    | "--read-pointblank"
                    | "--diagnose-pointblank"
                    | "--verify-features"
                    | "--render-smoke"
            )),
            "Select a supported runtime diagnostic"
        );
        ensure!(
            forwarded.len() <= 4,
            "Runtime diagnostic has too many arguments"
        );
        let staged = bundle::stage_game("pointblank")?;
        let executable = if let Some(stage) = &staged {
            staged_executable(stage)?
        } else {
            let games = catalog::load()?;
            let game = games
                .iter()
                .find(|game| game.id == "pointblank")
                .context("PointBlank runtime is absent")?;
            catalog::verify_runtime(game)?
        };
        let root = staged.as_ref().map(|stage| stage.root().to_path_buf());
        external_runtime::launch::run_diagnostic(&executable, forwarded)?;
        drop(staged);
        if let Some(root) = root {
            ensure!(
                removed(&root)?,
                "Runtime diagnostic temporary cleanup failed"
            );
        }
        return Ok(true);
    }
    Ok(false)
}

fn staged_executable(stage: &bundle::Staged) -> Result<PathBuf> {
    let games = catalog::load_from(&stage.root().join("games"))?;
    let game = games
        .iter()
        .find(|game| game.id == "pointblank")
        .context("Staged PointBlank catalog entry is absent")?;
    catalog::verify_runtime(game)
}
