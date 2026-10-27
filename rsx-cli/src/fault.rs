use crate::inspect::list_segments;
use crate::output::Failure;
use crate::output::Outcome;
use crate::procs;
use crate::procs::Record;
use crate::procs::Store;
use crate::segment::Frame;
use crate::segment::SegmentReader;
use rsx_cast::header::WalHeader;
use serde_json::json;
use serde_json::Value;
use std::fs::OpenOptions;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

#[cfg(test)]
#[path = "fault_test.rs"]
mod fault_test;

/// The byte `wal-corrupt` flips by default: the 9th byte of the first
/// record's payload, which the record's CRC covers.
pub const DEFAULT_CORRUPT_OFFSET: u64 = 24;

const MAX_IFACE_LEN: usize = 15;

/// SIGKILLs a process the cluster started. The record stays, so `cluster ps`
/// shows the process as exited until something restarts it.
pub fn kill(root: &Path, name: &str, is_json: bool) -> Outcome {
    let record = do_kill(root, name)?;
    if is_json {
        println!(
            "{}",
            json!({ "name": record.name, "pid": record.pid, "signal": "SIGKILL" })
        );
    } else {
        println!("killed {} (pid {}) with SIGKILL", record.name, record.pid);
    }
    Ok(())
}

pub(crate) fn do_kill(root: &Path, name: &str) -> Result<Record, Failure> {
    let Some(record) = Store::new(root).load(name)? else {
        return Err(Failure::runtime(format!(
            "{name}: no record under {}: only a process started by `cluster up` can be killed",
            root.join("tmp/cluster/pids").display()
        )));
    };
    if !procs::kill(&record) {
        return Err(Failure::runtime(format!(
            "{name}: not running (pid {} has exited)",
            record.pid
        )));
    }
    if record.is_running() {
        return Err(Failure::runtime(format!(
            "{name}: pid {} is still running after SIGKILL",
            record.pid
        )));
    }
    Ok(record)
}

/// A netem delay and loss on an interface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shaping {
    pub delay_ms: u32,
    pub loss_pct: f64,
}

/// Adds a netem discipline (`Some`) to the root of `iface`, or removes the
/// root discipline (`None`), through `sudo tc`.
pub fn shape_net(iface: &str, shaping: Option<Shaping>, is_json: bool) -> Outcome {
    check_iface(iface)?;
    if let Some(shaping) = &shaping {
        check_shaping(shaping)?;
    }
    run_tc(&tc_args(iface, shaping.as_ref()))?;
    if is_json {
        println!("{}", net_json(iface, shaping));
    } else if let Some(shaping) = shaping {
        println!(
            "{iface}: +{}ms delay, {}% loss",
            shaping.delay_ms, shaping.loss_pct
        );
    } else {
        println!("{iface}: netem cleared");
    }
    Ok(())
}

pub(crate) fn net_json(iface: &str, shaping: Option<Shaping>) -> Value {
    json!({
        "iface": iface,
        "delay_ms": shaping.map(|shaping| shaping.delay_ms),
        "loss_pct": shaping.map(|shaping| shaping.loss_pct),
        "active": shaping.is_some(),
    })
}

pub(crate) fn tc_args(iface: &str, shaping: Option<&Shaping>) -> Vec<String> {
    let mut args: Vec<String> = ["qdisc", "add", "dev", iface, "root"]
        .map(String::from)
        .into();
    match shaping {
        Some(shaping) => args.extend([
            "netem".to_string(),
            "delay".to_string(),
            format!("{}ms", shaping.delay_ms),
            "loss".to_string(),
            format!("{}%", shaping.loss_pct),
        ]),
        None => args[1] = "del".to_string(),
    }
    args
}

pub(crate) fn check_iface(iface: &str) -> Result<(), Failure> {
    let is_name = !iface.is_empty()
        && iface.len() <= MAX_IFACE_LEN
        && !iface.starts_with('-')
        && iface
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if !is_name {
        return Err(Failure::misuse(format!(
            "invalid interface name: {iface:?}"
        )));
    }
    Ok(())
}

pub(crate) fn check_shaping(shaping: &Shaping) -> Result<(), Failure> {
    if !(0.0..=100.0).contains(&shaping.loss_pct) {
        return Err(Failure::misuse(format!(
            "--loss-pct must be 0 to 100, got {}",
            shaping.loss_pct
        )));
    }
    if shaping.delay_ms == 0 && shaping.loss_pct == 0.0 {
        return Err(Failure::misuse(
            "a fault with no delay and no loss does nothing; use --clear to remove one",
        ));
    }
    Ok(())
}

fn run_tc(args: &[String]) -> Outcome {
    let output = Command::new("sudo")
        .args(["-n", "tc"])
        .args(args)
        .output()
        .map_err(|error| Failure::runtime(format!("failed to run sudo tc: {error}")))?;
    if !output.status.success() {
        return Err(Failure::runtime(format!(
            "tc {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

/// The byte `wal-corrupt` changed.
#[derive(Debug, PartialEq, Eq)]
pub struct Flip {
    pub stream_id: u32,
    pub file: PathBuf,
    pub offset: u64,
    pub old_byte: u8,
    pub new_byte: u8,
}

/// Flips the byte at `offset` of the newest segment of a stream, so
/// `wal verify` counts one CRC error. The offset must lie in the payload of
/// a complete record.
pub fn corrupt_wal(wal_dir: &Path, stream_id: u32, offset: u64, is_json: bool) -> Outcome {
    let flip = flip_byte(wal_dir, stream_id, offset)?;
    if is_json {
        println!(
            "{}",
            json!({
                "stream_id": flip.stream_id,
                "file": flip.file.display().to_string(),
                "offset": flip.offset,
                "old_byte": flip.old_byte,
                "new_byte": flip.new_byte,
            })
        );
    } else {
        println!(
            "flipped byte {} of {}: {:#04x} -> {:#04x}",
            flip.offset,
            flip.file.display(),
            flip.old_byte,
            flip.new_byte
        );
    }
    Ok(())
}

pub(crate) fn flip_byte(wal_dir: &Path, stream_id: u32, offset: u64) -> Result<Flip, Failure> {
    let segments = list_segments(stream_id, wal_dir)?;
    let Some(newest) = segments.last() else {
        return Err(Failure::runtime(format!(
            "stream {stream_id} has no segment under {}",
            wal_dir.display()
        )));
    };
    let file = newest.path.clone();
    let failed = |error: std::io::Error| {
        Failure::runtime(format!(
            "failed to flip a byte of {}: {error}",
            file.display()
        ))
    };
    if !is_payload_byte(&file, offset).map_err(failed)? {
        return Err(Failure::runtime(format!(
            "byte {offset} of {} is not inside the payload of a complete record",
            file.display()
        )));
    }
    let mut handle = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&file)
        .map_err(failed)?;
    handle.seek(SeekFrom::Start(offset)).map_err(failed)?;
    let mut byte = [0u8; 1];
    handle.read_exact(&mut byte).map_err(failed)?;
    let old_byte = byte[0];
    let new_byte = !old_byte;
    handle.seek(SeekFrom::Start(offset)).map_err(failed)?;
    handle.write_all(&[new_byte]).map_err(failed)?;
    handle.sync_data().map_err(failed)?;
    Ok(Flip {
        stream_id,
        file,
        offset,
        old_byte,
        new_byte,
    })
}

/// True when `offset` lies in the payload of a complete record of the
/// segment; a flip there fails that record's CRC and nothing else.
fn is_payload_byte(file: &Path, offset: u64) -> std::io::Result<bool> {
    let mut reader = SegmentReader::open(file)?;
    loop {
        let start = reader.offset;
        let Some(Frame::Record { .. }) = reader.read_frame()? else {
            return Ok(false);
        };
        if offset < reader.offset {
            return Ok(offset >= start + WalHeader::SIZE as u64);
        }
    }
}
