use rsx_cast::wal::oldest_and_highest_seq;
use rsx_cast::wal::WalWriter;
use std::io;
use std::path::Path;

/// Open mark's WAL writer, numbering new records after the highest seq
/// already on disk (rotated segments and the active file). An empty WAL
/// starts at seq 1. The writer is opened first so a torn tail is cut
/// before the active file is scanned.
pub fn open_wal_writer(
    stream_id: u32,
    wal_dir: &Path,
    max_file_size: u64,
) -> io::Result<WalWriter> {
    let mut writer = WalWriter::new(stream_id, wal_dir, max_file_size)?;
    if let Some((_, highest)) = oldest_and_highest_seq(stream_id, wal_dir)? {
        writer.set_next_seq(highest + 1);
    }
    Ok(writer)
}

#[cfg(test)]
#[path = "wal_test.rs"]
mod wal_test;
