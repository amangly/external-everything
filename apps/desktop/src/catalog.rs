use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

const MAX_CATALOG: u64 = 1024 * 1024;
const MAX_RUNTIME: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Game {
    pub id: String,
    pub name: String,
    pub executable: Option<PathBuf>,
    pub sha256: Option<String>,
    pub embedded: bool,
}

impl Game {
    pub fn available(&self) -> bool {
        (self.embedded || self.executable.is_some()) && self.sha256.is_some()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    schema: u32,
    games: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: String,
    name: String,
    executable: Option<PathBuf>,
    sha256: Option<String>,
}

pub fn root() -> Result<PathBuf> {
    Ok(std::env::current_exe()?
        .parent()
        .context("Application directory is absent")?
        .join("games"))
}

pub fn load() -> Result<Vec<Game>> {
    if let Some(bytes) = crate::bundle::catalog_bytes() {
        return parse_embedded(bytes);
    }
    load_from(&root()?)
}

fn parse_embedded(bytes: &[u8]) -> Result<Vec<Game>> {
    let mut games = parse(Path::new("embedded"), bytes)?;
    for game in &mut games {
        game.embedded = game.executable.take().is_some();
    }
    Ok(games)
}

pub fn load_from(directory: &Path) -> Result<Vec<Game>> {
    let catalog = directory.join("catalog.json");
    hyperrev_loader::files::reject_links(&catalog)?;
    if !catalog.try_exists()? {
        return Ok(Vec::new());
    }
    let file = fs::File::open(&catalog)?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= MAX_CATALOG,
        "Game catalog exceeds its size limit"
    );
    let mut bytes = Vec::new();
    file.take(MAX_CATALOG + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_CATALOG,
        "Game catalog exceeds its size limit"
    );
    parse(directory, &bytes)
}

fn parse(directory: &Path, bytes: &[u8]) -> Result<Vec<Game>> {
    let catalog: Catalog = serde_json::from_slice(bytes).context("Read game catalog")?;
    ensure!(
        catalog.schema == 1 && catalog.games.len() <= 128,
        "Unsupported game catalog"
    );
    let mut ids = std::collections::BTreeSet::new();
    let mut games = Vec::new();
    for entry in catalog.games {
        ensure!(
            !entry.id.is_empty()
                && entry.id.len() <= 64
                && entry
                    .id
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'),
            "Invalid game identifier"
        );
        ensure!(ids.insert(entry.id.clone()), "Duplicate game identifier");
        ensure!(
            !entry.name.trim().is_empty()
                && entry.name.len() <= 128
                && !entry.name.chars().any(char::is_control),
            "Invalid game name"
        );
        ensure!(
            entry.executable.is_some() == entry.sha256.is_some(),
            "Game executable and SHA-256 require each other"
        );
        let executable = if let Some(relative) = entry.executable {
            ensure!(
                !relative.as_os_str().is_empty()
                    && relative
                        .components()
                        .all(|part| matches!(part, Component::Normal(_))),
                "Game executable must stay inside its games directory"
            );
            ensure!(
                relative
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("exe")),
                "External runtime must be an executable"
            );
            let destination = directory.join(relative);
            // The path is lexical metadata here. Filesystem link and identity
            // checks run on the actual staged/external executable before launch.
            Some(destination)
        } else {
            None
        };
        if let Some(hash) = &entry.sha256 {
            ensure!(
                hash.len() == 64
                    && hash
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "Invalid game executable SHA-256"
            );
        }
        games.push(Game {
            id: entry.id,
            name: entry.name,
            executable,
            sha256: entry.sha256,
            embedded: false,
        });
    }
    Ok(games)
}

pub fn verify_runtime(game: &Game) -> Result<PathBuf> {
    ensure!(
        !game.embedded,
        "Embedded runtime requires owned staging before launch"
    );
    let path = game
        .executable
        .as_ref()
        .context("This game's external runtime is pending")?;
    let expected = game
        .sha256
        .as_ref()
        .context("External runtime identity is absent")?;
    hyperrev_loader::files::reject_links(path)?;
    let file = fs::File::open(path)
        .with_context(|| format!("Open external runtime {}", path.display()))?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_RUNTIME,
        "External runtime exceeds its size limit"
    );
    let mut bytes = Vec::new();
    file.take(MAX_RUNTIME + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 == metadata.len() && bytes.len() as u64 <= MAX_RUNTIME,
        "External runtime changed while reading"
    );
    ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == *expected,
        "External runtime SHA-256 differs from its catalog"
    );
    ensure!(
        bytes.len() >= 64 && &bytes[..2] == b"MZ",
        "External runtime requires a Windows PE image"
    );
    let pe = u32::from_le_bytes(bytes[60..64].try_into()?) as usize;
    let end = pe
        .checked_add(26)
        .context("External runtime PE offset overflow")?;
    ensure!(
        end <= bytes.len() && &bytes[pe..pe + 4] == b"PE\0\0",
        "External runtime PE header is invalid"
    );
    ensure!(
        u16::from_le_bytes(bytes[pe + 4..pe + 6].try_into()?) == 0x8664,
        "External runtime requires x64"
    );
    Ok(path.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn directory() -> PathBuf {
        std::env::current_dir()
            .unwrap()
            .join("target/catalog-fixture")
    }
    #[test]
    fn an_empty_catalog_is_valid() {
        assert!(
            parse(&directory(), br#"{"schema":1,"games":[]}"#)
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn embedded_catalog_advertises_runtime_without_a_sibling_path() {
        let json = serde_json::json!({"schema":1,"games":[{"id":"pointblank","name":"PointBlank","executable":"pointblank/external-runtime.exe","sha256":"a".repeat(64)}]});
        let games = parse_embedded(&serde_json::to_vec(&json).unwrap()).unwrap();
        assert!(games[0].available() && games[0].embedded);
        assert!(games[0].executable.is_none());
        assert!(verify_runtime(&games[0]).is_err());
        let pending =
            parse_embedded(br#"{"schema":1,"games":[{"id":"future","name":"Future"}]}"#).unwrap();
        assert!(!pending[0].available() && !pending[0].embedded);
    }

    #[test]
    fn future_games_have_a_name_without_a_fake_runtime() {
        let games = parse(
            &directory(),
            br#"{"schema":1,"games":[{"id":"example","name":"Example"}]}"#,
        )
        .unwrap();
        assert!(!games[0].available());
    }
    #[test]
    fn catalog_paths_stay_beneath_games() {
        for path in ["../outside.exe", "C:/outside.exe", "/outside.exe"] {
            let json = serde_json::json!({"schema":1,"games":[{"id":"example","name":"Example","executable":path,"sha256":"a".repeat(64)}]});
            assert!(parse(&directory(), &serde_json::to_vec(&json).unwrap()).is_err());
        }
    }
    #[test]
    fn duplicate_games_are_rejected() {
        assert!(
            parse(
                &directory(),
                br#"{"schema":1,"games":[{"id":"same","name":"One"},{"id":"same","name":"Two"}]}"#
            )
            .is_err()
        );
    }
    #[test]
    fn executable_and_identity_are_an_atomic_contract() {
        assert!(parse(&directory(), br#"{"schema":1,"games":[{"id":"example","name":"Example","executable":"example.exe"}]}"#).is_err());
    }
    #[test]
    fn unknown_catalog_fields_are_rejected() {
        assert!(parse(&directory(), br#"{"schema":1,"games":[],"ignore":true}"#).is_err());
    }
}
