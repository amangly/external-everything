use crate::{
    catalog::{self, Game},
    probe,
};
use anyhow::{Context, Result, ensure};
use hyperrev_loader::{app, boot::Firmware, files, platform};
use hyperrev_transport::Readiness;
use std::process::{Child, Command, ExitStatus, Stdio};

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
    app::perform(app::Operation::Setup, app::default_input())?;
    // A successful EFI installation is distinct from attachment readiness.
    inspect()
}

pub fn restore() -> Result<Snapshot> {
    app::perform(app::Operation::Restore, app::default_input())?;
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
    child: Child,
}

impl RuntimeSession {
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        self.child.try_wait().context("Check external runtime")
    }
}

impl Drop for RuntimeSession {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub fn launch(game_id: &str) -> Result<RuntimeSession> {
    require_attachment(&probe::check()?)?;
    let games = catalog::load()?;
    let game = games
        .iter()
        .find(|game| game.id == game_id)
        .context("Game is absent from the installed catalog")?;
    let executable = catalog::verify_runtime(game)?;
    let directory = executable
        .parent()
        .context("External runtime directory is absent")?;
    // Recheck after filesystem validation; a cached UI status never grants launch.
    require_attachment(&probe::check()?)?;
    let child = Command::new(&executable)
        .current_dir(directory)
        .arg("--game")
        .arg(&game.id)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Open {} external window", game.name))?;
    Ok(RuntimeSession {
        game_id: game.id.clone(),
        child,
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
