use crate::check::Check;
use crate::check::Report;
use crate::check::Tally;
use crate::check::BBO;
use crate::check::COMPLETION;
use crate::check::DATABASE;
use crate::check::FIFO;
use crate::check::FILLS;
use crate::check::FUNDING;
use crate::check::INTACT;
use crate::check::LOCKS;
use crate::check::POSITIONS;
use crate::check::REPLAY_CROSSED;
use crate::check::REPLAY_MATCH;
use crate::check::SEQS;
use crate::check::SLAB;
use crate::check::SPSC;
use crate::check::TIPS;
use crate::inspect;
use crate::ledger::Ledger;
use crate::output::Failure;
use crate::output::Format;
use crate::output::Outcome;
use crate::persisted::check_positions;
use crate::persisted::check_tips;
use crate::persisted::Persisted;
use crate::pg::Pg;
use crate::replay;
use crate::replay::Replayed;
use crate::stream_reader::StreamReader;
use rsx_cast::wal::extract_seq;
use serde_json::json;
use std::path::Path;
use std::path::PathBuf;

const NO_DATABASE: &str = "no database url (pass --db or set DATABASE_URL)";

pub struct Options {
    pub wal_dirs: Vec<PathBuf>,
    /// Only these streams; empty means every stream of every directory.
    pub only: Vec<u32>,
    pub database_url: Option<String>,
}

enum Database {
    Absent,
    Unreachable(String),
    Connected(Box<Pg>),
}

fn list_streams(options: &Options) -> Result<Vec<(PathBuf, u32)>, Failure> {
    let mut streams = Vec::new();
    for wal_dir in &options.wal_dirs {
        for stream_id in inspect::find_streams(wal_dir, &[])? {
            if options.only.is_empty() || options.only.contains(&stream_id) {
                streams.push((wal_dir.clone(), stream_id));
            }
        }
    }
    for wanted in &options.only {
        if !streams.iter().any(|(_, stream_id)| stream_id == wanted) {
            return Err(Failure::runtime(format!(
                "no wal directory for stream {} under any given wal dir",
                wanted
            )));
        }
    }
    Ok(streams)
}

/// Reads the stream from its first record, or up to `max_seq`, into a ledger.
pub(crate) fn scan_stream<'a>(
    wal_dir: &Path,
    stream_id: u32,
    max_seq: Option<u64>,
    persisted: Option<&'a Persisted>,
) -> Result<Ledger<'a>, Failure> {
    let mut ledger = Ledger::new(wal_dir, stream_id, persisted);
    let mut reader = StreamReader::open(stream_id, 0, wal_dir)?;
    loop {
        match reader.next_record() {
            Ok(Some(raw)) => {
                let past_end = max_seq
                    .zip(extract_seq(&raw.payload))
                    .is_some_and(|(max, seq)| seq > max);
                if past_end {
                    break;
                }
                ledger.apply(raw.header.record_type, &raw.payload);
            }
            Ok(None) => break,
            Err(error) => {
                ledger.note_read_error(error.message());
                break;
            }
        }
    }
    ledger.finish();
    Ok(ledger)
}

fn check_intact(streams: &[(PathBuf, u32)], ledgers: &[Ledger]) -> Result<Check, Failure> {
    let mut tally = Tally::default();
    for (wal_dir, stream_id) in streams {
        let verdict = inspect::verify_stream(*stream_id, wal_dir)?;
        tally.checked += verdict.records;
        if verdict.is_corrupt() {
            let mut violation = verdict.to_json();
            if let Some(map) = violation.as_object_mut() {
                map.insert("wal_dir".to_string(), json!(wal_dir.to_string_lossy()));
                map.insert(
                    "message".to_string(),
                    json!("crc error, sequence gap or unreadable segment"),
                );
            }
            tally.add_violation(violation);
        }
    }
    for ledger in ledgers {
        tally.merge(&ledger.intact);
    }
    Ok(Check::from_tally(None, INTACT, "records", tally))
}

fn check_or_explain(
    invariant: u8,
    name: &'static str,
    noun: &str,
    tally: Tally,
    reasons: &[String],
) -> Check {
    let mut check = Check::from_tally(Some(invariant), name, noun, tally);
    if check.checked == 0 && !reasons.is_empty() {
        check.detail = reasons.join("; ");
    }
    check
}

fn replay_stream<'a>(
    pg: &Pg,
    wal_dir: &Path,
    ledger: Ledger<'a>,
    persisted: Option<&'a Persisted>,
) -> Result<(Ledger<'a>, Replayed, Vec<String>), Failure> {
    let stream_id = ledger.stream_id();
    let mut replayed = Replayed::default();
    let boot = match pg.load_boot_config(stream_id, replay::book_capacity(&ledger)) {
        Ok(Some(boot)) => boot,
        Ok(None) => {
            let reason = format!("stream {}: no symbol config in postgres", stream_id);
            return Ok((ledger, replayed, vec![reason]));
        }
        Err(error) => {
            let reason = format!("stream {}: symbol config: {}", stream_id, error);
            return Ok((ledger, replayed, vec![reason]));
        }
    };
    let recovered = match replay::recover_book(boot, wal_dir) {
        Ok(recovered) => recovered,
        Err(error) => {
            replayed.matches.checked += 1;
            replayed.matches.add_violation(json!({
                "stream_id": stream_id,
                "message": format!("replay failed: {}", error),
            }));
            return Ok((ledger, replayed, Vec::new()));
        }
    };
    let replayed_seq = recovered.next_seq.saturating_sub(1);
    let ledger = if replayed_seq == ledger.last_seq {
        ledger
    } else {
        scan_stream(wal_dir, stream_id, Some(replayed_seq), persisted)?
    };
    let (replayed, skip) = replay::judge(&ledger, &recovered);
    Ok((ledger, replayed, skip.into_iter().collect()))
}

struct Scan<'a> {
    ledgers: Vec<Ledger<'a>>,
    tip_ends: Vec<(u32, u64)>,
    replayed: Replayed,
    replay_reasons: Vec<String>,
}

impl Scan<'_> {
    fn note_reason(&mut self, reason: String) {
        if !self.replay_reasons.contains(&reason) {
            self.replay_reasons.push(reason);
        }
    }
}

fn open_database(options: &Options) -> Database {
    match options.database_url.as_deref() {
        None => Database::Absent,
        Some(url) => match Pg::connect(url) {
            Ok(pg) => Database::Connected(Box::new(pg)),
            Err(error) => Database::Unreachable(error),
        },
    }
}

fn scan_streams<'a>(
    streams: &[(PathBuf, u32)],
    database: &Database,
    persisted: Option<&'a Persisted>,
) -> Result<Scan<'a>, Failure> {
    let mut scan = Scan {
        ledgers: Vec::new(),
        tip_ends: Vec::new(),
        replayed: Replayed::default(),
        replay_reasons: Vec::new(),
    };
    for (wal_dir, stream_id) in streams {
        let mut ledger = scan_stream(wal_dir, *stream_id, None, persisted)?;
        if ledger.has_order_flow() {
            scan.tip_ends.push((*stream_id, ledger.last_seq));
            match database {
                Database::Connected(pg) => {
                    let (rescanned, verdict, reasons) =
                        replay_stream(pg, wal_dir, ledger, persisted)?;
                    ledger = rescanned;
                    scan.replayed.merge(verdict);
                    for reason in reasons {
                        scan.note_reason(reason);
                    }
                }
                Database::Absent => scan.note_reason(NO_DATABASE.to_string()),
                Database::Unreachable(_) => scan.note_reason("postgres unreachable".to_string()),
            }
        }
        scan.ledgers.push(ledger);
    }
    Ok(scan)
}

fn check_database(database: &Database) -> Check {
    match database {
        Database::Absent => Check::skip(None, DATABASE, NO_DATABASE),
        Database::Unreachable(error) => Check::fail(None, DATABASE, error),
        Database::Connected(_) => Check::pass(None, DATABASE, "connected"),
    }
}

/// Runs every check over the WAL directories and, with a database url,
/// over Postgres.
pub fn run(options: &Options) -> Result<Report, Failure> {
    let streams = list_streams(options)?;
    let database = open_database(options);
    let persisted = match &database {
        Database::Connected(pg) => Some(pg.load_persisted()),
        _ => None,
    };
    let persisted_ok = persisted.as_ref().and_then(|result| result.as_ref().ok());
    let scan = scan_streams(&streams, &database, persisted_ok)?;

    let mut fills = Tally::default();
    let mut completion = Tally::default();
    let mut fifo = Tally::default();
    let mut monotonic = Tally::default();
    let mut bbo = Tally::default();
    for ledger in &scan.ledgers {
        fills.merge(&ledger.fills_before_done);
        completion.merge(&ledger.completion);
        fifo.merge(&ledger.fifo);
        monotonic.merge(&ledger.monotonic);
        bbo.merge(&ledger.bbo);
    }
    let reasons = &scan.replay_reasons;
    let (positions, tips) =
        check_persisted(&database, persisted.as_ref(), &scan.ledgers, &scan.tip_ends);
    let (funding, locks) = match &database {
        Database::Connected(pg) => (pg.check_funding(), pg.check_locks()),
        other => (skip_for(other, 9, FUNDING), skip_for(other, 10, LOCKS)),
    };
    let checks = vec![
        check_intact(&streams, &scan.ledgers)?,
        check_database(&database),
        Check::from_tally(Some(1), FILLS, "fill-order pairs", fills),
        Check::from_tally(Some(2), COMPLETION, "orders", completion),
        Check::from_tally(Some(3), FIFO, "fills against resting orders", fifo),
        check_or_explain(
            3,
            REPLAY_MATCH,
            "replayed books",
            scan.replayed.matches,
            reasons,
        ),
        positions,
        Check::from_tally(Some(5), SEQS, "records", monotonic),
        tips,
        Check::from_tally(Some(6), BBO, "two-sided BBO records", bbo),
        check_or_explain(
            6,
            REPLAY_CROSSED,
            "replayed books",
            scan.replayed.crossed,
            reasons,
        ),
        Check::skip(
            Some(7),
            SPSC,
            "in-process ring; nothing outside the process records its order",
        ),
        check_or_explain(8, SLAB, "replayed books", scan.replayed.slab, reasons),
        funding,
        locks,
    ];
    Ok(Report { checks })
}

fn skip_for(database: &Database, invariant: u8, name: &'static str) -> Check {
    match database {
        Database::Unreachable(_) => Check::skip(Some(invariant), name, "postgres unreachable"),
        _ => Check::skip(Some(invariant), name, NO_DATABASE),
    }
}

fn check_persisted(
    database: &Database,
    persisted: Option<&Result<Persisted, String>>,
    ledgers: &[Ledger],
    tip_ends: &[(u32, u64)],
) -> (Check, Check) {
    match persisted {
        Some(Ok(persisted)) => (
            check_positions(persisted, ledgers),
            check_tips(persisted, tip_ends),
        ),
        Some(Err(error)) => (
            Check::fail(Some(4), POSITIONS, error),
            Check::fail(Some(5), TIPS, error),
        ),
        None => (
            skip_for(database, 4, POSITIONS),
            skip_for(database, 5, TIPS),
        ),
    }
}

/// Runs the checks and prints the report. Any failed check makes the command
/// a `Runtime` failure after the report is printed.
pub fn verify(options: &Options, format: &Format) -> Outcome {
    let report = run(options)?;
    match format {
        Format::Json => println!("{}", report.to_json()),
        Format::Text(_) => println!("{}", report.to_text()),
    }
    if !report.is_failed() {
        return Ok(());
    }
    Err(Failure::runtime(format!(
        "verify failed: {} of {} checks",
        report.failed_count(),
        report.checks.len()
    )))
}
