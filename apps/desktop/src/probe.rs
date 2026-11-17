use anyhow::{Context, Result, bail, ensure};
use hyperrev_transport::Readiness;
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const LIMIT: u64 = 4096;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireStatus {
    schema: u32,
    response: u64,
    cr3: u64,
}

pub fn internal() -> Result<()> {
    let status = match hyperrev_transport::probe()? {
        Readiness::Ready { cr3 } => WireStatus {
            schema: 1,
            response: 0x1337,
            cr3,
        },
        Readiness::Absent { response } => WireStatus {
            schema: 1,
            response,
            cr3: 0,
        },
    };
    println!("{}", serde_json::to_string(&status)?);
    Ok(())
}

pub fn check() -> Result<Readiness> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--internal-attachment-probe")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().context("Start isolated attachment check")?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let result = (|| -> Result<Readiness> {
        loop {
            if let Some(status) = child.try_wait()? {
                ensure!(
                    status.success(),
                    "Attachment probe exited with {status}; game launch stays disabled"
                );
                break;
            }
            if Instant::now() >= deadline {
                bail!("Attachment probe timed out; retry the check");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child
            .stdout
            .take()
            .context("Attachment probe output is absent")?;
        let mut bytes = Vec::new();
        output.take(LIMIT + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= LIMIT,
            "Attachment probe response exceeds its limit"
        );
        decode(&bytes)
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

fn decode(bytes: &[u8]) -> Result<Readiness> {
    let status: WireStatus =
        serde_json::from_slice(bytes).context("Read attachment probe response")?;
    ensure!(status.schema == 1, "Attachment probe protocol differs");
    hyperrev_transport::classify(status.response, status.cr3)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_successful_child_response_still_requires_a_live_cr3() {
        assert!(decode(br#"{"schema":1,"response":4919,"cr3":0}"#).is_err());
    }
    #[test]
    fn absence_survives_the_child_boundary() {
        assert_eq!(
            decode(br#"{"schema":1,"response":32,"cr3":0}"#).unwrap(),
            Readiness::Absent { response: 32 }
        );
    }
    #[test]
    fn valid_live_child_response_reaches_ready() {
        assert_eq!(
            decode(br#"{"schema":1,"response":4919,"cr3":4096}"#).unwrap(),
            Readiness::Ready { cr3: 4096 }
        );
    }
    #[test]
    fn malformed_child_output_is_rejected() {
        assert!(decode(b"not-json").is_err());
    }
}
