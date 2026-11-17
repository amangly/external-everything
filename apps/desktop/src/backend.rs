use crate::{
    bundle,
    catalog::{self, Game},
    probe,
};
use anyhow::{Context, Result, ensure};
use hyperrev_loader::{app, boot::Firmware, files, platform};
use hyperrev_transport::Readiness;
use std::process::ExitStatus;

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub attachment: Readiness,
    pub installed: bool,
    pub games: Vec<Game>,
}

fn installed() -> Result<bool> {
    let journal_path = app::default_input().journal;
    if !journal_path.try_exists()? {
        return Ok(false);
    }
    let journal = files::load(&journal_path)?;
    let firmware = platform::NativeFirmware::open()?;
    Ok(
        firmware.get(&journal.boot_name)?.as_ref() == Some(&journal.created_option)
            && firmware.get(&journal.safe_name)?.as_ref() == Some(&journal.safe_option)
            && firmware.get("BootOrder")?.as_ref() == Some(&journal.boot_order_after),
    )
}

pub fn inspect() -> Result<Snapshot> {
    let attachment = probe::check().context("Check hyper-reV attachment")?;
    let installed = matches!(attachment, Readiness::Ready { .. }) || installed()?;
    Ok(Snapshot {
        attachment,
        installed,
        games: catalog::load()?,
    })
}

pub fn setup() -> Result<Snapshot> {
    let staged = bundle::stage_boot()?;
    let mut input = app::default_input();
    if let Some(stage) = &staged {
        input.source = crate::recovery::retain(stage.root())?;
    }
    app::perform(app::Operation::Setup, input)?;
    // A successful EFI installation is distinct from attachment readiness.
    inspect()
}

pub fn restore() -> Result<Snapshot> {
    let input = app::default_input();
    let journal = files::load(&input.journal)?;
    app::perform(app::Operation::Restore, input)?;
    crate::recovery::cleanup(&journal)?;
    inspect()
}

fn require_attachment(readiness: &Readiness) -> Result<()> {
    ensure!(
        matches!(readiness, Readiness::Ready { cr3 } if cr3 & 0x000f_ffff_ffff_f000 != 0),
        "Attachment is inactive; start Windows through hyper-reV, then reopen External Everything"
    );
    Ok(())
}

pub struct RuntimeSession {
    pub game_id: String,
    runtime: external_runtime::launch::OwnedRuntime,
    // Declaration order drops the child before deleting its staged executable.
    _staged: Option<bundle::Staged>,
}

impl RuntimeSession {
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        self.runtime.try_wait()
    }
    pub fn pid(&self) -> u32 {
        self.runtime.pid()
    }
    pub fn staging_root(&self) -> Option<&std::path::Path> {
        self._staged.as_ref().map(bundle::Staged::root)
    }
}

pub fn launch(game_id: &str) -> Result<RuntimeSession> {
    require_attachment(&probe::check()?)?;
    let games = catalog::load()?;
    let game = games
        .iter()
        .find(|game| game.id == game_id)
        .context("Game is absent from the installed catalog")?;
    let staged = if game.embedded {
        bundle::stage_game(game_id)?
    } else {
        None
    };
    let executable = if let Some(stage) = &staged {
        let staged_games = catalog::load_from(&stage.root().join("games"))?;
        let staged_game = staged_games
            .iter()
            .find(|candidate| candidate.id == game.id)
            .context("Staged game is absent from its embedded catalog")?;
        ensure!(
            staged_game.sha256 == game.sha256,
            "Staged game identity differs from embedded metadata"
        );
        catalog::verify_runtime(staged_game)?
    } else {
        ensure!(!game.embedded, "Embedded game staging is unavailable");
        catalog::verify_runtime(game)?
    };
    // A cached UI status never grants launch. The runtime then validates its
    // contract and acknowledges both process startup and its actual control window.
    require_attachment(&probe::check()?)?;
    let runtime = external_runtime::launch::start(&executable, &game.id)
        .with_context(|| format!("Open {} external window", game.name))?;
    Ok(RuntimeSession {
        game_id: game.id.clone(),
        runtime,
        _staged: staged,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inactive_attachment_never_grants_a_game_launch() {
        assert!(require_attachment(&Readiness::Absent { response: 0 }).is_err());
    }
    #[test]
    fn invalid_ready_values_stay_out_of_the_launch_gate() {
        assert!(require_attachment(&Readiness::Ready { cr3: 0 }).is_err());
        assert!(require_attachment(&Readiness::Ready { cr3: 0xfff }).is_err());
    }
    #[test]
    fn verified_attachment_grants_a_game_launch_gate() {
        assert!(require_attachment(&Readiness::Ready { cr3: 0x1000 }).is_ok());
    }
}
