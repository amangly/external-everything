use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

pub const SCHEMA: u32 = 1;
pub const BACKEND_ABI: &str = "hyper-rev/cpuid-8a3b55c7927/v1";
pub const STARTED_EVENT: &str = "EXTERNAL_EVERYTHING_STARTED_EVENT";
pub const READY_EVENT: &str = "EXTERNAL_EVERYTHING_READY_EVENT";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contract {
    pub schema: u32,
    pub name: String,
    pub version: String,
    pub backend_abi: String,
    pub games: Vec<GameContract>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameContract {
    pub id: String,
    pub name: String,
    pub process_names: Vec<String>,
    pub pointer_width: u8,
    pub offset_build: String,
}

pub fn current() -> Contract {
    Contract {
        schema: SCHEMA,
        name: "external-runtime".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        backend_abi: BACKEND_ABI.into(),
        games: vec![GameContract {
            id: "pointblank".into(),
            name: "PointBlank".into(),
            process_names: vec!["PointBlank.exe".into()],
            pointer_width: 32,
            offset_build: "pb-id-20261115,pb-id-20261116".into(),
        }],
    }
}

pub fn validate(bytes: &[u8], game: &str) -> Result<Contract> {
    ensure!(
        bytes.len() <= 64 * 1024,
        "Runtime contract exceeds its size limit"
    );
    let contract: Contract =
        serde_json::from_slice(bytes).context("Read external runtime contract")?;
    ensure!(
        contract.schema == SCHEMA && contract.name == "external-runtime",
        "External runtime contract identity differs"
    );
    ensure!(
        contract.version == env!("CARGO_PKG_VERSION"),
        "External runtime version differs from the launcher"
    );
    ensure!(
        contract.backend_abi == BACKEND_ABI,
        "External runtime hypercall ABI differs"
    );
    ensure!(
        contract.games.len() <= 128,
        "External runtime contract contains too many games"
    );
    let matching: Vec<_> = contract
        .games
        .iter()
        .filter(|entry| entry.id == game)
        .collect();
    ensure!(
        matching.len() == 1,
        "Selected game is absent or duplicated in the runtime contract"
    );
    ensure!(
        matching[0].pointer_width == 32 || matching[0].pointer_width == 64,
        "Runtime pointer width is unsupported"
    );
    ensure!(
        !matching[0].offset_build.is_empty() && !matching[0].process_names.is_empty(),
        "Runtime game metadata is incomplete"
    );
    Ok(contract)
}

/// Child signals startup and renderer readiness separately. Readiness requires
/// successful ESP/menu paint passes, rather than that PointBlank has joined a match.
pub fn signal(variable: &str) -> Result<()> {
    let Some(name) = std::env::var_os(variable) else {
        return Ok(());
    };
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
        use windows_sys::Win32::System::Threading::{EVENT_MODIFY_STATE, OpenEventW, SetEvent};
        let text = name.to_string_lossy();
        ensure!(
            text.starts_with(r"Local\ExternalEverything-runtime-")
                && text.len() <= 200
                && !text.contains('\0'),
            "Runtime startup event name differs"
        );
        let wide: Vec<_> = name.encode_wide().chain([0]).collect();
        let handle = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, wide.as_ptr()) };
        ensure!(
            !handle.is_null(),
            "Open runtime startup event failed: {}",
            unsafe { GetLastError() }
        );
        let result = unsafe { SetEvent(handle) };
        let error = unsafe { GetLastError() };
        unsafe {
            CloseHandle(handle);
        }
        ensure!(result != 0, "Signal runtime startup event failed: {error}");
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = name;
        anyhow::bail!("Runtime launch events require Windows")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bytes(contract: &Contract) -> Vec<u8> {
        serde_json::to_vec(contract).unwrap()
    }
    #[test]
    fn the_pointblank_contract_is_self_consistent() {
        validate(&bytes(&current()), "pointblank").unwrap();
    }
    #[test]
    fn another_backend_abi_is_rejected() {
        let mut contract = current();
        contract.backend_abi = "palepale-other-abi".into();
        assert!(validate(&bytes(&contract), "pointblank").is_err());
    }
    #[test]
    fn absent_games_and_version_mismatches_are_rejected() {
        assert!(validate(&bytes(&current()), "missing").is_err());
        let mut contract = current();
        contract.version = "different".into();
        assert!(validate(&bytes(&contract), "pointblank").is_err());
    }
    #[test]
    fn duplicate_game_contracts_are_rejected() {
        let mut contract = current();
        contract.games.push(contract.games[0].clone());
        assert!(validate(&bytes(&contract), "pointblank").is_err());
    }
}
