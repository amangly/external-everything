//! Explicit read-only evidence capture for a selected PointBlank process.
//! This path never admits a build to production offset capture.
use anyhow::{Context, Result, ensure};
use hyperrev_transport::Connection;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Range {
    label: String,
    address: u64,
    size: usize,
}

pub fn read_ranges(pid: u32, request: &Path, output: &Path) -> Result<Value> {
    let data = fs::read(request)?;
    ensure!(
        data.len() <= 65536,
        "Diagnostic range request exceeds its bound"
    );
    let ranges: Vec<Range> = serde_json::from_slice(&data)?;
    ensure!(
        ranges.len() <= 512
            && ranges
                .iter()
                .try_fold(0usize, |sum, range| sum.checked_add(range.size))
                .is_some_and(|sum| sum <= 1024 * 1024),
        "Diagnostic range batch exceeds its bound"
    );
    let connection = Connection::open()?;
    let process = hyperrev_transport::process_by_pid(&connection, pid)?
        .context("Diagnostic PID is absent")?;
    ensure!(
        process.name.eq_ignore_ascii_case("PointBlank.exe"),
        "Diagnostic PID belongs to a different executable"
    );
    let mut results = Vec::new();
    for range in ranges {
        ensure!(
            range.label.len() <= 128
                && range.size <= 65536
                && range.address >= 65536
                && range
                    .address
                    .checked_add(range.size as u64)
                    .is_some_and(|end| end <= 0x1_0000_0000),
            "Invalid diagnostic read range"
        );
        let mut bytes = vec![0; range.size];
        let record = match connection.read(process.cr3, range.address, &mut bytes) {
            Ok(()) => {
                json!({"label":range.label,"address":range.address,"size":range.size,"hex":bytes.iter().map(|byte|format!("{byte:02x}")).collect::<String>()})
            }
            Err(error) => {
                json!({"label":range.label,"address":range.address,"size":range.size,"error":format!("{error:#}")})
            }
        };
        results.push(record);
    }
    let evidence = json!({"schema":1,"mode":"diagnostic-read-only","pid":pid,"cr3":process.cr3,"image_base":process.image_base,"image_size":process.image_size,"reads":results});
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    file.write_all(&serde_json::to_vec_pretty(&evidence)?)?;
    file.sync_all()?;
    Ok(json!({"pid":pid,"output":output,"ranges":evidence["reads"].as_array().map(Vec::len)}))
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        bytes
            .get(offset..offset + 2)
            .context("Truncated diagnostic PE field")?
            .try_into()?,
    ))
}
fn u32_at(bytes: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .context("Truncated diagnostic PE field")?
            .try_into()?,
    ))
}

pub fn capture(pid: u32, directory: &Path) -> Result<Value> {
    let connection = Connection::open()?;
    let process = hyperrev_transport::process_by_pid(&connection, pid)?
        .context("PointBlank diagnostic PID is absent")?;
    ensure!(
        process.name.eq_ignore_ascii_case("PointBlank.exe"),
        "Diagnostic PID belongs to a different executable"
    );
    ensure!(
        process.image_base <= u32::MAX as u64 && process.image_size <= 64 * 1024 * 1024,
        "Diagnostic image bounds differ from PointBlank PE32"
    );
    ensure!(
        !directory.exists(),
        "Choose a fresh PointBlank diagnostic directory"
    );
    let mut header = vec![0; 4096];
    connection.read(process.cr3, process.image_base, &mut header)?;
    ensure!(
        &header[..2] == b"MZ",
        "Diagnostic image has a missing DOS signature"
    );
    let nt = u32_at(&header, 60)? as usize;
    ensure!(
        header.get(nt..nt + 4) == Some(b"PE\0\0")
            && u16_at(&header, nt + 4)? == 0x14c
            && u16_at(&header, nt + 24)? == 0x10b,
        "Diagnostic image is not x86 PE32"
    );
    let section_count = u16_at(&header, nt + 6)? as usize;
    let optional = u16_at(&header, nt + 20)? as usize;
    ensure!(
        section_count <= 32 && optional <= 512,
        "Diagnostic PE section table exceeds its bound"
    );
    let table = nt
        .checked_add(24 + optional)
        .context("Diagnostic section table overflow")?;
    fs::create_dir_all(directory)?;
    let mut sections = Vec::new();
    for index in 0..section_count {
        let section = table + index * 40;
        let name = header
            .get(section..section + 8)
            .context("Truncated diagnostic section name")?;
        let end = name
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(name.len());
        let name = String::from_utf8_lossy(&name[..end]).into_owned();
        let rva = u32_at(&header, section + 12)?;
        let size = u32_at(&header, section + 8)?;
        ensure!(
            rva.checked_add(size)
                .is_some_and(|end| end <= process.image_size),
            "Diagnostic section exceeds SizeOfImage"
        );
        let mut record = json!({"name":name,"rva":rva,"size":size});
        if matches!(name.as_str(), ".text" | ".rdata" | ".data") && size > 0 {
            let mut bytes = vec![0; size as usize];
            let mut gaps = Vec::new();
            for (page, output) in bytes.chunks_mut(4096).enumerate() {
                let address = process.image_base + u64::from(rva) + (page * 4096) as u64;
                if let Err(error) = connection.read(process.cr3, address, output) {
                    output.fill(0);
                    gaps.push(json!({"offset":page*4096,"error":format!("{error:#}")}));
                }
            }
            let filename = format!("section-{index}.bin");
            let path = directory.join(&filename);
            let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            record["file"] = json!(filename);
            record["sha256"] = json!(format!("{:x}", Sha256::digest(&bytes)));
            record["read_gaps"] = json!(gaps);
        }
        sections.push(record);
    }
    let path = external_runtime::engine::target_image_path(pid)?;
    let disk = fs::read(&path)?;
    ensure!(
        disk.len() <= 128 * 1024 * 1024,
        "Diagnostic disk image exceeds its bound"
    );
    let evidence = json!({"schema":1,"mode":"diagnostic-only","pid":pid,"eprocess":process.eprocess,"cr3":process.cr3,"image_base":process.image_base,"image_size":process.image_size,"path":path,"disk_sha256":format!("{:x}",Sha256::digest(&disk)),"sections":sections});
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("evidence.json"))?;
    file.write_all(&serde_json::to_vec_pretty(&evidence)?)?;
    file.sync_all()?;
    Ok(evidence)
}
