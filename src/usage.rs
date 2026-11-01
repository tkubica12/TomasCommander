//! Local-only usage log: which commands are used and how they were invoked, unhandled keys,
//! navigation, cancelled dialogs, failures and slow work, for periodic personal analysis.
//!
//! Recording never blocks and never touches the disk on the caller's thread: an enabled log
//! builds one small [`Event`] and `try_send`s it into a bounded channel; a full channel only
//! increments a dropped counter. One writer thread appends JSON Lines to
//! `usage\YYYY-MM-DD.jsonl` (local date) beside the preferences file, flushing every few seconds
//! or every few hundred events. Nothing is ever sent anywhere. See `docs\decisions\usage-log.md`.

use crate::files::{self, FileResult};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub mod report;

/// Events buffered between the UI and the writer thread; recording never waits for space.
pub const CHANNEL_CAPACITY: usize = 4096;
/// Buffered lines are written at least this often.
pub const FLUSH_INTERVAL: Duration = Duration::from_secs(5);
/// ... or after this many events, whichever comes first.
pub const FLUSH_EVENTS: usize = 256;
/// Per-day file cap; further events of that day are not written.
pub const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
/// Cap of all day files together.
pub const MAX_TOTAL_BYTES: u64 = 200 * 1024 * 1024;
/// Day files older than this are deleted when the writer starts.
pub const RETENTION_DAYS: i64 = 90;
/// Longest wait for the final flush when the application exits.
pub const SHUTDOWN_WAIT: Duration = Duration::from_millis(300);
/// Aggregated counters are written this often (and at session end).
pub const SUMMARY_INTERVAL: Duration = Duration::from_secs(300);
/// No input for this long counts as idle time.
pub const IDLE_AFTER: Duration = Duration::from_secs(300);

const DAY_MS: i64 = 86_400_000;
const RESERVED: [&str; 4] = ["t", "tz", "s", "k"];

/// The usage folder for a preferences file: `<settings dir>\usage`.
pub fn folder_for_settings(settings: &Path) -> PathBuf {
    settings
        .parent()
        .map(Path::to_owned)
        .unwrap_or_default()
        .join("usage")
}

pub fn unix_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

pub fn now_ms() -> u64 {
    unix_ms(SystemTime::now())
}

/// One field value. Static strings avoid allocation for names chosen by the program.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(&'static str),
    Text(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Null,
    /// Named counters, written as a JSON object.
    Counts(Vec<(String, u64)>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(value) => Some(value),
            Self::Text(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(value) => Some(*value),
            _ => None,
        }
    }

    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Self::Str(value) => push_json_str(out, value),
            Self::Text(value) => push_json_str(out, value),
            Self::Int(value) => push_display(out, value),
            Self::Float(value) if value.is_finite() => {
                let _ = serde_json::to_writer(&mut *out, value);
            }
            Self::Float(_) | Self::Null => out.extend_from_slice(b"null"),
            Self::Bool(value) => out.extend_from_slice(if *value { b"true" } else { b"false" }),
            Self::Counts(counts) => {
                out.push(b'{');
                for (index, (name, count)) in counts.iter().enumerate() {
                    if index > 0 {
                        out.push(b',');
                    }
                    push_json_str(out, name);
                    out.push(b':');
                    push_display(out, count);
                }
                out.push(b'}');
            }
        }
    }
}

impl From<&'static str> for Value {
    fn from(value: &'static str) -> Self {
        Self::Str(value)
    }
}
impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}
impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}
impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}
impl From<i32> for Value {
    fn from(value: i32) -> Self {
        Self::Int(value.into())
    }
}
impl From<u32> for Value {
    fn from(value: u32) -> Self {
        Self::Int(value.into())
    }
}
impl From<u64> for Value {
    fn from(value: u64) -> Self {
        Self::Int(i64::try_from(value).unwrap_or(i64::MAX))
    }
}
impl From<usize> for Value {
    fn from(value: usize) -> Self {
        Self::Int(i64::try_from(value).unwrap_or(i64::MAX))
    }
}
impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}
impl From<f32> for Value {
    fn from(value: f32) -> Self {
        Self::Float(f64::from(value))
    }
}
/// Durations are recorded as whole milliseconds.
impl From<Duration> for Value {
    fn from(value: Duration) -> Self {
        Self::Int(i64::try_from(value.as_millis()).unwrap_or(i64::MAX))
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(value: Option<T>) -> Self {
        value.map_or(Self::Null, Into::into)
    }
}

/// One usage record. `kind` and field names are stable identifiers (see the decision record).
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub at_ms: u64,
    pub kind: &'static str,
    pub fields: Vec<(&'static str, Value)>,
}

impl Event {
    pub fn new(kind: &'static str) -> Self {
        Self::at(now_ms(), kind)
    }

    pub fn at(at_ms: u64, kind: &'static str) -> Self {
        Self {
            at_ms,
            kind,
            fields: Vec::with_capacity(8),
        }
    }

    #[must_use]
    pub fn with(mut self, key: &'static str, value: impl Into<Value>) -> Self {
        debug_assert!(!RESERVED.contains(&key), "{key} is a reserved usage field");
        self.fields.push((key, value.into()));
        self
    }

    pub fn field(&self, key: &str) -> Option<&Value> {
        self.fields
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value)
    }

    pub fn text(&self, key: &str) -> Option<&str> {
        self.field(key).and_then(Value::as_str)
    }

    /// `{"t":<unix ms>,"tz":<local offset minutes>,"s":"<session>","k":"<kind>",...}\n`
    pub fn write_line(&self, out: &mut Vec<u8>, session: &str, tz_minutes: i32) {
        out.extend_from_slice(b"{\"t\":");
        push_display(out, self.at_ms);
        out.extend_from_slice(b",\"tz\":");
        push_display(out, tz_minutes);
        out.extend_from_slice(b",\"s\":");
        push_json_str(out, session);
        out.extend_from_slice(b",\"k\":");
        push_json_str(out, self.kind);
        for (key, value) in &self.fields {
            out.push(b',');
            push_json_str(out, key);
            out.push(b':');
            value.write(out);
        }
        out.extend_from_slice(b"}\n");
    }

    pub fn to_line(&self, session: &str, tz_minutes: i32) -> String {
        let mut out = Vec::new();
        self.write_line(&mut out, session, tz_minutes);
        String::from_utf8(out).unwrap_or_default()
    }
}

fn push_json_str(out: &mut Vec<u8>, value: &str) {
    let _ = serde_json::to_writer(&mut *out, value);
}

fn push_display(out: &mut Vec<u8>, value: impl std::fmt::Display) {
    let _ = write!(out, "{value}");
}

/// Captured events of an in-memory log (tests).
pub type Captured = Arc<Mutex<Vec<Event>>>;

enum Message {
    Event(Event),
    Flush(SyncSender<()>),
    Delete(SyncSender<FileResult<usize>>),
    Wake,
}

enum Sink {
    Channel(SyncSender<Message>),
    Memory(Captured),
}

struct Inner {
    enabled: AtomicBool,
    dropped: AtomicU64,
    sink: Sink,
    session: String,
    folder: Option<PathBuf>,
    bytes: Arc<AtomicU64>,
    problem: Arc<Mutex<Option<String>>>,
    shutdown: Arc<AtomicBool>,
    done: Mutex<Option<Receiver<()>>>,
}

/// Cheap, cloneable recording handle. The default handle records nothing.
#[derive(Clone, Default)]
pub struct UsageLog {
    inner: Option<Arc<Inner>>,
}

impl UsageLog {
    /// A log that never records (tests, fixture runs).
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Starts the writer thread for `config.dir`. Housekeeping (retention, total cap) runs on
    /// that thread; nothing here touches the disk.
    pub fn start(config: Config, enabled: bool) -> Self {
        let (sender, receiver) = mpsc::sync_channel(config.capacity.max(1));
        let (done_sender, done) = mpsc::sync_channel(1);
        let session = session_id();
        let bytes = Arc::new(AtomicU64::new(0));
        let problem = Arc::new(Mutex::new(None));
        let shutdown = Arc::new(AtomicBool::new(false));
        let writer = Writer::new(
            config.clone(),
            session.clone(),
            bytes.clone(),
            problem.clone(),
            shutdown.clone(),
        );
        let started = thread::Builder::new()
            .name("usage-log".into())
            .spawn(move || writer.run(receiver, done_sender));
        if let Err(error) = started
            && let Ok(mut problem) = problem.lock()
        {
            *problem = Some(format!("Could not start the usage writer: {error}"));
        }
        Self {
            inner: Some(Arc::new(Inner {
                enabled: AtomicBool::new(enabled),
                dropped: AtomicU64::new(0),
                sink: Sink::Channel(sender),
                session,
                folder: Some(config.dir),
                bytes,
                problem,
                shutdown,
                done: Mutex::new(Some(done)),
            })),
        }
    }

    /// An enabled log that keeps events in memory instead of writing them (tests).
    pub fn memory() -> (Self, Captured) {
        let events = Captured::default();
        let log = Self {
            inner: Some(Arc::new(Inner {
                enabled: AtomicBool::new(true),
                dropped: AtomicU64::new(0),
                sink: Sink::Memory(events.clone()),
                session: "memory".into(),
                folder: None,
                bytes: Arc::default(),
                problem: Arc::default(),
                shutdown: Arc::default(),
                done: Mutex::new(None),
            })),
        };
        (log, events)
    }

    /// Records the event built by `build`, only when enabled. Never blocks or performs I/O.
    #[inline]
    pub fn record(&self, build: impl FnOnce() -> Event) {
        let Some(inner) = &self.inner else {
            return;
        };
        if !inner.enabled.load(Ordering::Relaxed) {
            return;
        }
        let event = build();
        match &inner.sink {
            Sink::Channel(sender) => {
                if sender.try_send(Message::Event(event)).is_err() {
                    inner.dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
            Sink::Memory(events) => {
                if let Ok(mut events) = events.lock() {
                    events.push(event);
                }
            }
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.inner
            .as_ref()
            .is_some_and(|inner| inner.enabled.load(Ordering::Relaxed))
    }

    /// Turns recording on or off; a disabled handle stays disabled.
    pub fn set_enabled(&self, enabled: bool) {
        if let Some(inner) = &self.inner
            && !inner.shutdown.load(Ordering::Acquire)
        {
            inner.enabled.store(enabled, Ordering::Relaxed);
        }
    }

    /// Whether this run has a usage folder (false for disabled and in-memory logs).
    pub fn folder(&self) -> Option<&Path> {
        self.inner
            .as_ref()
            .and_then(|inner| inner.folder.as_deref())
    }

    pub fn session(&self) -> &str {
        self.inner
            .as_ref()
            .map_or("", |inner| inner.session.as_str())
    }

    /// Events lost because the channel was full or the writer had stopped.
    pub fn dropped(&self) -> u64 {
        self.inner
            .as_ref()
            .map_or(0, |inner| inner.dropped.load(Ordering::Relaxed))
    }

    /// Bytes of day files in the usage folder, as known to the writer.
    pub fn stored_bytes(&self) -> u64 {
        self.inner
            .as_ref()
            .map_or(0, |inner| inner.bytes.load(Ordering::Relaxed))
    }

    /// The latest writer problem, such as an unwritable or linked usage folder.
    pub fn problem(&self) -> Option<String> {
        self.inner.as_ref().and_then(|inner| {
            inner
                .problem
                .lock()
                .ok()
                .and_then(|problem| problem.clone())
        })
    }

    fn channel(&self) -> Option<&SyncSender<Message>> {
        match &self.inner.as_ref()?.sink {
            Sink::Channel(sender) => Some(sender),
            Sink::Memory(_) => None,
        }
    }

    /// Writes buffered events now and waits up to `timeout` (background threads and tests only).
    pub fn flush(&self, timeout: Duration) -> bool {
        let Some(sender) = self.channel() else {
            return true;
        };
        let deadline = Instant::now() + timeout;
        let (ack, done) = mpsc::sync_channel(1);
        let mut message = Message::Flush(ack);
        loop {
            match sender.try_send(message) {
                Ok(()) => break,
                Err(TrySendError::Full(returned)) if Instant::now() < deadline => {
                    message = returned;
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => return false,
            }
        }
        done.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .is_ok()
    }

    /// Asks the writer to delete this app's usage files (day logs and exported reports only).
    /// The receiver yields the number of deleted files.
    pub fn delete_all(&self) -> FileResult<Receiver<FileResult<usize>>> {
        let sender = self
            .channel()
            .ok_or("This run has no usage folder; nothing to delete.")?;
        let (ack, done) = mpsc::sync_channel(1);
        sender
            .try_send(Message::Delete(ack))
            .map_err(|_| "The usage writer is busy; try again in a moment.")?;
        Ok(done)
    }

    /// Stops recording, writes what was recorded and waits at most `timeout`; never hangs.
    pub fn shutdown(&self, timeout: Duration) -> bool {
        let Some(inner) = &self.inner else {
            return true;
        };
        let Sink::Channel(sender) = &inner.sink else {
            return true;
        };
        inner.enabled.store(false, Ordering::Relaxed);
        inner.shutdown.store(true, Ordering::Release);
        // A full channel needs no wake-up: the writer checks the flag after every message.
        let _ = sender.try_send(Message::Wake);
        let done = inner.done.lock().ok().and_then(|mut done| done.take());
        match done {
            Some(done) => !matches!(done.recv_timeout(timeout), Err(RecvTimeoutError::Timeout)),
            None => true,
        }
    }
}

fn session_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(0);
    let mixed = (nanos ^ (u64::from(std::process::id()) << 32)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    format!("{mixed:016x}")
}

/// Writer settings; [`Config::new`] uses the documented limits.
#[derive(Clone, Debug)]
pub struct Config {
    pub dir: PathBuf,
    pub capacity: usize,
    pub flush_interval: Duration,
    pub flush_events: usize,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub retention_days: i64,
    /// Fixed local offset in minutes (tests); `None` asks Windows.
    pub tz_minutes: Option<i32>,
}

impl Config {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            capacity: CHANNEL_CAPACITY,
            flush_interval: FLUSH_INTERVAL,
            flush_events: FLUSH_EVENTS,
            max_file_bytes: MAX_FILE_BYTES,
            max_total_bytes: MAX_TOTAL_BYTES,
            retention_days: RETENTION_DAYS,
            tz_minutes: None,
        }
    }
}

/// Local UTC offset in minutes at `at_ms` (current Windows time-zone rules).
pub fn local_offset_minutes(at_ms: u64) -> i32 {
    #[cfg(windows)]
    {
        use windows::Win32::{Foundation::FILETIME, Storage::FileSystem::FileTimeToLocalFileTime};
        let ticks = at_ms
            .saturating_add(11_644_473_600_000)
            .saturating_mul(10_000);
        let utc = FILETIME {
            dwLowDateTime: ticks as u32,
            dwHighDateTime: (ticks >> 32) as u32,
        };
        let mut local = FILETIME::default();
        if unsafe { FileTimeToLocalFileTime(&utc, &mut local) }.is_ok() {
            let local = (u64::from(local.dwHighDateTime) << 32) | u64::from(local.dwLowDateTime);
            return ((local as i64 - ticks as i64) / 600_000_000) as i32;
        }
        0
    }
    #[cfg(not(windows))]
    {
        let _ = at_ms;
        0
    }
}

/// Days since 1970-01-01 in local time.
pub fn local_day(at_ms: u64, tz_minutes: i32) -> i64 {
    (i64::try_from(at_ms).unwrap_or(i64::MAX) + i64::from(tz_minutes) * 60_000).div_euclid(DAY_MS)
}

/// Gregorian (year, month, day) of a day number (days since 1970-01-01).
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

/// Day number (days since 1970-01-01) of a Gregorian date.
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let mp = i64::from(if month > 2 { month - 3 } else { month + 9 });
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `YYYY-MM-DD` of a day number.
pub fn date_text(day: i64) -> String {
    let (year, month, day) = civil_from_days(day);
    format!("{year:04}-{month:02}-{day:02}")
}

pub fn day_file_name(day: i64) -> String {
    format!("{}.jsonl", date_text(day))
}

fn parse_date(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
    {
        return None;
    }
    let year = text[0..4].parse().ok()?;
    let month = text[5..7].parse().ok()?;
    let day = text[8..10].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let number = days_from_civil(year, month, day);
    (civil_from_days(number) == (year, month, day)).then_some(number)
}

/// The day of a `YYYY-MM-DD.jsonl` usage file name; anything else is not ours.
pub fn parse_day_file(name: &str) -> Option<i64> {
    parse_date(name.strip_suffix(".jsonl")?)
}

/// Whether `name` is an exported `report-YYYY-MM-DD.md`.
pub fn is_report_file(name: &str) -> bool {
    name.strip_prefix("report-")
        .and_then(|rest| rest.strip_suffix(".md"))
        .and_then(parse_date)
        .is_some()
}

/// Rejects a usage folder that is a link/junction or not a folder; `Ok(false)` when missing.
pub fn check_folder(dir: &Path) -> FileResult<bool> {
    match fs::symlink_metadata(dir) {
        Ok(metadata) if files::reparse(&metadata) || !metadata.is_dir() => Err(format!(
            "The usage folder {} is a link or not a folder; usage logging is off.",
            files::display_path(dir)
        )),
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "Inspect usage folder {}: {error}",
            files::display_path(dir)
        )),
    }
}

/// Our regular (non-link) files in `dir` accepted by `owned`, with their sizes.
fn owned_files(
    dir: &Path,
    owned: impl Fn(&str) -> bool,
) -> FileResult<Vec<(String, PathBuf, u64)>> {
    let mut found = Vec::new();
    let entries = fs::read_dir(dir)
        .map_err(|error| format!("Read usage folder {}: {error}", files::display_path(dir)))?;
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !owned(&name) {
            continue;
        }
        let path = entry.path();
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() && !files::reparse(&metadata) => {
                found.push((name, path, metadata.len()));
            }
            _ => {}
        }
    }
    Ok(found)
}

struct OpenFile {
    day: i64,
    writer: BufWriter<File>,
    size: u64,
}

struct Writer {
    config: Config,
    session: String,
    bytes: Arc<AtomicU64>,
    problem: Arc<Mutex<Option<String>>>,
    shutdown: Arc<AtomicBool>,
    file: Option<OpenFile>,
    capped_day: Option<i64>,
    total_capped: bool,
    refused: bool,
    pending: usize,
    last_flush: Instant,
    line: Vec<u8>,
    tz: Option<(i32, Instant)>,
}

impl Writer {
    fn new(
        config: Config,
        session: String,
        bytes: Arc<AtomicU64>,
        problem: Arc<Mutex<Option<String>>>,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        Self {
            config,
            session,
            bytes,
            problem,
            shutdown,
            file: None,
            capped_day: None,
            total_capped: false,
            refused: false,
            pending: 0,
            last_flush: Instant::now(),
            line: Vec::with_capacity(512),
            tz: None,
        }
    }

    fn run(mut self, receiver: Receiver<Message>, done: SyncSender<()>) {
        if let Err(error) = self.housekeeping(now_ms()) {
            self.report(error);
        }
        loop {
            let wait = if self.pending > 0 {
                self.config
                    .flush_interval
                    .saturating_sub(self.last_flush.elapsed())
            } else {
                Duration::from_secs(3600)
            };
            match receiver.recv_timeout(wait) {
                Ok(message) => self.handle(message),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if self.pending > 0
                && (self.pending >= self.config.flush_events
                    || self.last_flush.elapsed() >= self.config.flush_interval)
            {
                self.flush();
            }
            if self.shutdown.load(Ordering::Acquire) {
                // Write what was recorded before shutdown; recording has stopped.
                for _ in 0..self.config.capacity {
                    match receiver.try_recv() {
                        Ok(message) => self.handle(message),
                        Err(_) => break,
                    }
                }
                break;
            }
        }
        self.flush();
        self.file = None;
        let _ = done.try_send(());
    }

    fn handle(&mut self, message: Message) {
        match message {
            Message::Event(event) => self.write(&event),
            Message::Flush(ack) => {
                self.flush();
                let _ = ack.try_send(());
            }
            Message::Delete(ack) => {
                let result = self.delete_all();
                let _ = ack.try_send(result);
            }
            Message::Wake => {}
        }
    }

    fn report(&self, problem: String) {
        if let Ok(mut current) = self.problem.lock() {
            *current = Some(problem);
        }
    }

    fn tz(&mut self) -> i32 {
        if let Some(fixed) = self.config.tz_minutes {
            return fixed;
        }
        match self.tz {
            Some((offset, checked)) if checked.elapsed() < Duration::from_secs(60) => offset,
            _ => {
                let offset = local_offset_minutes(now_ms());
                self.tz = Some((offset, Instant::now()));
                offset
            }
        }
    }

    /// Deletes day files beyond retention and, above the total cap, the oldest earlier days.
    fn housekeeping(&mut self, now_ms: u64) -> FileResult<usize> {
        match check_folder(&self.config.dir) {
            Ok(true) => {}
            Ok(false) => return Ok(0),
            Err(error) => {
                self.refused = true;
                return Err(error);
            }
        }
        let today = local_day(now_ms, self.tz());
        let mut removed = 0;
        let mut kept = Vec::new();
        for (name, path, size) in
            owned_files(&self.config.dir, |name| parse_day_file(name).is_some())?
        {
            let day = parse_day_file(&name).unwrap_or(today);
            if day < today - self.config.retention_days {
                if fs::remove_file(&path).is_ok() {
                    removed += 1;
                }
            } else {
                kept.push((day, path, size));
            }
        }
        let mut total: u64 = kept.iter().map(|(_, _, size)| size).sum();
        if total > self.config.max_total_bytes {
            kept.sort_by_key(|(day, _, _)| *day);
            for (day, path, size) in &kept {
                if total <= self.config.max_total_bytes {
                    break;
                }
                if *day < today && fs::remove_file(path).is_ok() {
                    total -= size;
                    removed += 1;
                }
            }
        }
        self.bytes.store(total, Ordering::Relaxed);
        Ok(removed)
    }

    fn open(&mut self, day: i64) -> FileResult<OpenFile> {
        let dir = &self.config.dir;
        fs::create_dir_all(dir).map_err(|error| {
            format!("Create usage folder {}: {error}", files::display_path(dir))
        })?;
        if let Err(error) = check_folder(dir) {
            self.refused = true;
            return Err(error);
        }
        let path = dir.join(day_file_name(day));
        if let Ok(metadata) = fs::symlink_metadata(&path)
            && (files::reparse(&metadata) || !metadata.is_file())
        {
            self.refused = true;
            return Err(format!(
                "Usage file {} is a link or not a file; usage logging is off.",
                files::display_path(&path)
            ));
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| format!("Open usage file {}: {error}", files::display_path(&path)))?;
        let size = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        Ok(OpenFile {
            day,
            writer: BufWriter::with_capacity(64 * 1024, file),
            size,
        })
    }

    fn write(&mut self, event: &Event) {
        if self.refused || self.total_capped {
            return;
        }
        let tz = self.tz();
        let day = local_day(event.at_ms, tz);
        if self.capped_day == Some(day) {
            return;
        }
        if self.file.as_ref().is_none_or(|file| file.day != day) {
            self.flush();
            self.file = None;
            match self.open(day) {
                Ok(file) => self.file = Some(file),
                Err(error) => {
                    self.report(error);
                    return;
                }
            }
        }
        self.line.clear();
        event.write_line(&mut self.line, &self.session, tz);
        let length = self.line.len() as u64;
        let size = self.file.as_ref().map_or(0, |file| file.size);
        if size + length > self.config.max_file_bytes {
            self.capped_day = Some(day);
            self.cap(event.at_ms, tz, "file", self.config.max_file_bytes);
        } else if self.bytes.load(Ordering::Relaxed) + length > self.config.max_total_bytes {
            self.total_capped = true;
            self.cap(event.at_ms, tz, "total", self.config.max_total_bytes);
        } else {
            self.append();
        }
    }

    /// Records once that a cap stopped writing, then flushes.
    fn cap(&mut self, at_ms: u64, tz: i32, scope: &'static str, limit: u64) {
        self.line.clear();
        Event::at(at_ms, "cap_reached")
            .with("scope", scope)
            .with("limit_bytes", limit)
            .write_line(&mut self.line, &self.session, tz);
        self.append();
        self.flush();
    }

    fn append(&mut self) {
        let Some(file) = &mut self.file else {
            return;
        };
        match file.writer.write_all(&self.line) {
            Ok(()) => {
                let length = self.line.len() as u64;
                file.size += length;
                self.bytes.fetch_add(length, Ordering::Relaxed);
                self.pending += 1;
            }
            Err(error) => {
                self.file = None;
                self.report(format!("Write usage file: {error}"));
            }
        }
    }

    fn flush(&mut self) {
        if let Some(file) = &mut self.file
            && let Err(error) = file.writer.flush()
        {
            self.file = None;
            self.report(format!("Write usage file: {error}"));
        }
        self.pending = 0;
        self.last_flush = Instant::now();
    }

    /// Deletes day logs and exported reports; other files are never touched.
    fn delete_all(&mut self) -> FileResult<usize> {
        self.flush();
        self.file = None;
        if !check_folder(&self.config.dir)? {
            self.bytes.store(0, Ordering::Relaxed);
            return Ok(0);
        }
        let mut removed = 0;
        let mut failures = Vec::new();
        let mut remaining = 0;
        for (name, path, size) in owned_files(&self.config.dir, |name| {
            parse_day_file(name).is_some() || is_report_file(name)
        })? {
            match fs::remove_file(&path) {
                Ok(()) => removed += 1,
                Err(error) => {
                    if parse_day_file(&name).is_some() {
                        remaining += size;
                    }
                    failures.push(format!("{name}: {error}"));
                }
            }
        }
        self.bytes.store(remaining, Ordering::Relaxed);
        self.capped_day = None;
        self.total_capped = false;
        if failures.is_empty() {
            Ok(removed)
        } else {
            Err(format!(
                "Deleted {removed} usage files; could not delete {}",
                failures.join("; ")
            ))
        }
    }
}

/// Aggregated counts (keys, characters, pointer use) written in periodic `summary` events
/// instead of one event per keystroke. Keys are static, so counting never allocates after the
/// first occurrence of a key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    counts: BTreeMap<(&'static str, &'static str), u64>,
}

impl Counters {
    pub fn add(&mut self, group: &'static str, name: &'static str, count: u64) {
        if count > 0 {
            *self.counts.entry((group, name)).or_insert(0) += count;
        }
    }

    pub fn bump(&mut self, group: &'static str, name: &'static str) {
        self.add(group, name, 1);
    }

    pub fn get(&self, group: &str, name: &str) -> u64 {
        self.counts
            .iter()
            .find(|((g, n), _)| *g == group && *n == name)
            .map_or(0, |(_, count)| *count)
    }

    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }

    /// The counts as `"group.name": count` and a reset.
    pub fn take(&mut self) -> Value {
        Value::Counts(
            std::mem::take(&mut self.counts)
                .into_iter()
                .map(|((group, name), count)| (format!("{group}.{name}"), count))
                .collect(),
        )
    }
}

/// Active versus idle time: gaps between inputs longer than [`IDLE_AFTER`] are idle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Activity {
    last_input: Instant,
    active: Duration,
    idle: Duration,
}

impl Activity {
    pub fn new(now: Instant) -> Self {
        Self {
            last_input: now,
            active: Duration::ZERO,
            idle: Duration::ZERO,
        }
    }

    pub fn input(&mut self, now: Instant) {
        let gap = now.saturating_duration_since(self.last_input);
        if gap > IDLE_AFTER {
            self.idle += gap;
        } else {
            self.active += gap;
        }
        self.last_input = now;
    }

    /// (active, idle) including the time since the last input.
    pub fn totals(&self, now: Instant) -> (Duration, Duration) {
        let gap = now.saturating_duration_since(self.last_input);
        if gap > IDLE_AFTER {
            (self.active, self.idle + gap)
        } else {
            (self.active + gap, self.idle)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "tc-usage-{name}-{}-{}",
                std::process::id(),
                now_ms()
            ));
            let _ = fs::remove_dir_all(&path);
            Self(path)
        }

        fn config(&self) -> Config {
            let mut config = Config::new(self.0.clone());
            config.tz_minutes = Some(0);
            config
        }

        fn read(&self, day: i64) -> String {
            fs::read_to_string(self.0.join(day_file_name(day))).unwrap_or_default()
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn channel_log(capacity: usize) -> (UsageLog, Receiver<Message>) {
        let (sender, receiver) = mpsc::sync_channel(capacity);
        let log = UsageLog {
            inner: Some(Arc::new(Inner {
                enabled: AtomicBool::new(true),
                dropped: AtomicU64::new(0),
                sink: Sink::Channel(sender),
                session: "test".into(),
                folder: None,
                bytes: Arc::default(),
                problem: Arc::default(),
                shutdown: Arc::default(),
                done: Mutex::new(None),
            })),
        };
        (log, receiver)
    }

    const DAY: u64 = 86_400_000;
    // 2026-10-31 12:00 UTC.
    const NOON: u64 = 1_793_448_000_000;

    #[test]
    fn events_serialize_with_stable_field_names() {
        let event = Event::at(1_700_000_000_123, "command")
            .with("cmd", "/copy")
            .with("via", "shortcut")
            .with("selected", 3usize)
            .with("focused", r#"C:\Users\Tomáš\a "b".txt"#.to_owned())
            .with("ok", true)
            .with("ratio", 0.5)
            .with("nan", f64::NAN)
            .with("missing", None::<String>)
            .with("counts", Value::Counts(vec![("key.Down".into(), 7)]));
        let line = event.to_line("abc", 60);
        assert_eq!(
            line,
            "{\"t\":1700000000123,\"tz\":60,\"s\":\"abc\",\"k\":\"command\",\"cmd\":\"/copy\",\"via\":\"shortcut\",\"selected\":3,\"focused\":\"C:\\\\Users\\\\Tomáš\\\\a \\\"b\\\".txt\",\"ok\":true,\"ratio\":0.5,\"nan\":null,\"missing\":null,\"counts\":{\"key.Down\":7}}\n"
        );
        let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["focused"], r#"C:\Users\Tomáš\a "b".txt"#);
        assert_eq!(event.text("cmd"), Some("/copy"));
        assert_eq!(event.field("selected").and_then(Value::as_i64), Some(3));
        assert_eq!(Value::from(Duration::from_millis(1500)), Value::Int(1500));
    }

    #[test]
    fn dates_round_trip_and_name_only_our_files() {
        for (day, text) in [
            (0, "1970-01-01"),
            (-1, "1969-12-31"),
            (20_757, "2026-10-31"),
            (11_016, "2000-02-29"),
        ] {
            assert_eq!(date_text(day), text);
            assert_eq!(parse_day_file(&format!("{text}.jsonl")), Some(day));
        }
        assert_eq!(local_day(NOON, 0), 20_757);
        assert_eq!(date_text(local_day(NOON, 0)), "2026-10-31");
        assert_eq!(
            local_day(NOON + 11 * 3_600_000 + 1, 60),
            local_day(NOON, 0) + 1
        );
        for name in [
            "2026-02-30.jsonl",
            "2026-13-01.jsonl",
            "notes.jsonl",
            "2026-10-31.json",
            "2026-10-31.jsonl.bak",
            "x2026-10-31.jsonl",
        ] {
            assert_eq!(parse_day_file(name), None, "{name}");
        }
        assert!(is_report_file("report-2026-10-31.md"));
        assert!(!is_report_file("report-latest.md"));
        assert_eq!(
            folder_for_settings(Path::new(r"C:\a\preferences.txt")),
            PathBuf::from(r"C:\a\usage")
        );
    }

    #[test]
    fn disabled_logs_build_nothing_and_never_write() {
        let log = UsageLog::disabled();
        log.record(|| unreachable!("a disabled log must not build events"));
        log.set_enabled(true);
        assert!(!log.is_enabled());
        assert!(log.folder().is_none());
        assert!(log.flush(Duration::from_millis(10)));
        assert!(log.shutdown(Duration::from_millis(10)));
        assert!(log.delete_all().is_err());

        let folder = Folder::new("off");
        let log = UsageLog::start(folder.config(), false);
        log.record(|| unreachable!("a switched-off log must not build events"));
        assert!(log.flush(Duration::from_secs(5)));
        assert!(!folder.0.exists(), "no folder is created without events");
        log.set_enabled(true);
        log.record(|| Event::at(NOON, "after"));
        assert!(log.shutdown(Duration::from_secs(5)));
        assert!(folder.read(local_day(NOON, 0)).contains("\"k\":\"after\""));
    }

    #[test]
    fn full_channel_drops_are_counted_without_blocking() {
        let (log, receiver) = channel_log(2);
        let started = Instant::now();
        for index in 0..10u64 {
            log.record(|| Event::at(index, "probe"));
        }
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(log.dropped(), 8);
        assert_eq!(receiver.try_iter().count(), 2);
        drop(receiver);
        log.record(|| Event::at(0, "after-writer-stopped"));
        assert_eq!(log.dropped(), 9);
    }

    #[test]
    fn writer_batches_flushes_on_count_and_on_shutdown() {
        let folder = Folder::new("batch");
        let mut config = folder.config();
        config.flush_events = 4;
        config.flush_interval = Duration::from_secs(3600);
        let log = UsageLog::start(config, true);
        let day = local_day(NOON, 0);
        for index in 0..3 {
            log.record(|| Event::at(NOON + index, "batched").with("index", index));
        }
        thread::sleep(Duration::from_millis(200));
        assert_eq!(
            folder.read(day).lines().count(),
            0,
            "three events stay buffered"
        );
        log.record(|| Event::at(NOON + 3, "batched").with("index", 3));
        let deadline = Instant::now() + Duration::from_secs(5);
        while folder.read(day).lines().count() < 4 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            folder.read(day).lines().count(),
            4,
            "the fourth event flushes"
        );
        log.record(|| Event::at(NOON + 4, "session_end"));
        assert!(log.shutdown(Duration::from_secs(5)));
        let text = folder.read(day);
        assert_eq!(text.lines().count(), 5);
        assert!(
            text.lines()
                .last()
                .unwrap()
                .contains("\"k\":\"session_end\"")
        );
        for line in text.lines() {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(value["s"], log.session());
            assert_eq!(value["tz"], 0);
        }
        assert_eq!(log.stored_bytes(), text.len() as u64);
        log.record(|| unreachable!("recording stops at shutdown"));
    }

    #[test]
    fn writer_flushes_after_the_interval() {
        let folder = Folder::new("interval");
        let mut config = folder.config();
        config.flush_interval = Duration::from_millis(50);
        let log = UsageLog::start(config, true);
        log.record(|| Event::at(NOON, "timed"));
        let deadline = Instant::now() + Duration::from_secs(5);
        while folder.read(local_day(NOON, 0)).is_empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(folder.read(local_day(NOON, 0)).contains("timed"));
        assert!(log.shutdown(Duration::from_secs(5)));
    }

    #[test]
    fn days_rotate_by_local_date() {
        let folder = Folder::new("rotate");
        let mut config = folder.config();
        config.tz_minutes = Some(120);
        let log = UsageLog::start(config, true);
        let late = NOON + 10 * 3_600_000 - 1; // 21:59:59.999 UTC = 23:59:59.999 local
        log.record(|| Event::at(late, "before-midnight"));
        log.record(|| Event::at(late + 1, "after-midnight"));
        log.record(|| Event::at(late + 1 + DAY, "next-day"));
        assert!(log.shutdown(Duration::from_secs(5)));
        let day = local_day(late, 120);
        assert!(folder.read(day).contains("before-midnight"));
        assert!(folder.read(day + 1).contains("after-midnight"));
        assert!(folder.read(day + 2).contains("next-day"));
        assert!(!folder.read(day).contains("after-midnight"));
        assert!(folder.read(day).contains("\"tz\":120"));
    }

    #[test]
    fn caps_stop_writing_once_and_record_it() {
        let folder = Folder::new("cap");
        let mut config = folder.config();
        config.max_file_bytes = 400;
        let log = UsageLog::start(config.clone(), true);
        for index in 0..50u64 {
            log.record(|| Event::at(NOON + index, "filler").with("index", index));
        }
        log.record(|| Event::at(NOON + DAY, "next-day"));
        assert!(log.shutdown(Duration::from_secs(5)));
        let day = local_day(NOON, 0);
        let text = folder.read(day);
        assert!(text.len() as u64 <= 400 + 200, "{}", text.len());
        assert_eq!(text.matches("cap_reached").count(), 1);
        assert!(text.lines().last().unwrap().contains("\"scope\":\"file\""));
        assert!(
            folder.read(day + 1).contains("next-day"),
            "a new day writes again"
        );

        let folder = Folder::new("total");
        let mut config = folder.config();
        config.max_total_bytes = 300;
        let log = UsageLog::start(config, true);
        for index in 0..20u64 {
            log.record(|| Event::at(NOON + index * DAY, "filler"));
        }
        assert!(log.shutdown(Duration::from_secs(5)));
        let all: String = (0..20).map(|offset| folder.read(day + offset)).collect();
        assert_eq!(all.matches("cap_reached").count(), 1);
        assert!(all.contains("\"scope\":\"total\""));
        assert!(all.len() <= 300 + 200);
    }

    #[test]
    fn retention_deletes_only_old_day_files() {
        let folder = Folder::new("retention");
        fs::create_dir_all(&folder.0).unwrap();
        let today = local_day(now_ms(), 0);
        let old = folder.0.join(day_file_name(today - 91));
        let edge = folder.0.join(day_file_name(today - 90));
        let recent = folder.0.join(day_file_name(today - 3));
        let other = folder.0.join("notes.txt");
        let report = folder
            .0
            .join(format!("report-{}.md", date_text(today - 120)));
        let lookalike = folder
            .0
            .join(format!("{}.jsonl.bak", date_text(today - 120)));
        let nested = folder
            .0
            .join(day_file_name(today - 100).replace(".jsonl", ""));
        for path in [&old, &edge, &recent, &other, &report, &lookalike] {
            fs::write(path, b"{}\n").unwrap();
        }
        fs::create_dir_all(nested.join("inner")).unwrap();
        let folder_named_like_a_log = folder.0.join(day_file_name(today - 100));
        fs::create_dir_all(&folder_named_like_a_log).unwrap();
        let log = UsageLog::start(folder.config(), true);
        assert!(log.flush(Duration::from_secs(5)));
        assert!(!old.exists());
        for path in [&edge, &recent, &other, &report, &lookalike] {
            assert!(path.exists(), "{}", path.display());
        }
        assert!(folder_named_like_a_log.is_dir());
        assert_eq!(log.stored_bytes(), 6);
        assert!(log.shutdown(Duration::from_secs(5)));
    }

    #[test]
    fn total_cap_at_startup_removes_the_oldest_earlier_days() {
        let folder = Folder::new("startup-cap");
        fs::create_dir_all(&folder.0).unwrap();
        let today = local_day(now_ms(), 0);
        for offset in [5, 4, 3, 0] {
            fs::write(
                folder.0.join(day_file_name(today - offset)),
                vec![b'x'; 100],
            )
            .unwrap();
        }
        let mut config = folder.config();
        config.max_total_bytes = 250;
        let log = UsageLog::start(config, true);
        assert!(log.flush(Duration::from_secs(5)));
        assert!(!folder.0.join(day_file_name(today - 5)).exists());
        assert!(!folder.0.join(day_file_name(today - 4)).exists());
        assert!(folder.0.join(day_file_name(today - 3)).exists());
        assert!(folder.0.join(day_file_name(today)).exists());
        assert_eq!(log.stored_bytes(), 200);
        assert!(log.shutdown(Duration::from_secs(5)));
    }

    #[cfg(windows)]
    #[test]
    fn linked_usage_folders_are_refused() {
        let folder = Folder::new("junction");
        let target = folder.0.join("target");
        let link = folder.0.join("usage");
        fs::create_dir_all(&target).unwrap();
        let old = target.join("2000-01-01.jsonl");
        fs::write(&old, b"{}\n").unwrap();
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .unwrap();
        assert!(status.status.success(), "{status:?}");
        let mut config = folder.config();
        config.dir = link.clone();
        let log = UsageLog::start(config, true);
        log.record(|| Event::at(NOON, "refused"));
        assert!(log.flush(Duration::from_secs(5)));
        assert!(old.exists(), "retention never follows a junction");
        assert!(!target.join(day_file_name(local_day(NOON, 0))).exists());
        assert!(log.problem().unwrap().contains("link"));
        assert!(log.delete_all().unwrap().recv().unwrap().is_err());
        assert!(old.exists());
        assert!(log.shutdown(Duration::from_secs(5)));
        fs::remove_dir(&link).unwrap();
    }

    #[test]
    fn delete_all_removes_only_usage_files_and_logging_continues() {
        let folder = Folder::new("delete");
        fs::create_dir_all(&folder.0).unwrap();
        let other = folder.0.join("keep.txt");
        fs::write(&other, b"keep").unwrap();
        fs::write(folder.0.join("report-2026-01-02.md"), b"report").unwrap();
        let log = UsageLog::start(folder.config(), true);
        log.record(|| Event::at(NOON, "before"));
        let deleted = log
            .delete_all()
            .unwrap()
            .recv_timeout(Duration::from_secs(5));
        assert_eq!(deleted.unwrap(), Ok(2));
        assert!(other.exists());
        assert_eq!(log.stored_bytes(), 0);
        log.record(|| Event::at(NOON, "after"));
        assert!(log.shutdown(Duration::from_secs(5)));
        let text = folder.read(local_day(NOON, 0));
        assert!(!text.contains("before"));
        assert!(text.contains("after"));
    }

    #[test]
    fn shutdown_is_bounded_and_idempotent() {
        let (log, _receiver) = channel_log(1);
        let (_done_sender, done) = mpsc::sync_channel::<()>(1);
        *log.inner.as_ref().unwrap().done.lock().unwrap() = Some(done);
        let started = Instant::now();
        assert!(
            !log.shutdown(Duration::from_millis(50)),
            "a stuck writer times out"
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(log.shutdown(Duration::from_millis(50)));
        log.set_enabled(true);
        assert!(!log.is_enabled(), "shutdown is final");
    }

    #[test]
    fn recording_overhead_is_tiny_for_the_caller() {
        let folder = Folder::new("overhead");
        let log = UsageLog::start(folder.config(), true);
        let started = Instant::now();
        for index in 0..100_000u64 {
            log.record(|| {
                Event::new("command")
                    .with("cmd", "/copy")
                    .with("via", "shortcut")
                    .with("index", index)
            });
        }
        let elapsed = started.elapsed();
        println!(
            "100k records: {elapsed:?} ({} dropped while the writer caught up)",
            log.dropped()
        );
        assert!(
            elapsed < Duration::from_millis(500),
            "100k records took {elapsed:?}"
        );
        assert!(log.shutdown(Duration::from_secs(10)));
        let disabled = UsageLog::start(folder.config(), false);
        let started = Instant::now();
        for _ in 0..1_000_000 {
            disabled.record(|| unreachable!());
        }
        assert!(started.elapsed() < Duration::from_millis(200));
        assert!(disabled.shutdown(Duration::from_secs(5)));
    }

    #[test]
    fn counters_and_activity_summarize_without_noise() {
        let mut counters = Counters::default();
        assert!(counters.is_empty());
        counters.bump("key", "Down");
        counters.bump("key", "Down");
        counters.add("typed", "filter", 5);
        counters.add("typed", "ask", 0);
        assert_eq!(counters.get("key", "Down"), 2);
        assert_eq!(
            counters.take(),
            Value::Counts(vec![("key.Down".into(), 2), ("typed.filter".into(), 5)])
        );
        assert!(counters.is_empty());

        let start = Instant::now();
        let mut activity = Activity::new(start);
        activity.input(start + Duration::from_secs(60));
        activity.input(start + Duration::from_secs(60 + 600));
        activity.input(start + Duration::from_secs(60 + 600 + 30));
        let (active, idle) = activity.totals(start + Duration::from_secs(60 + 600 + 30 + 10));
        assert_eq!(active, Duration::from_secs(100));
        assert_eq!(idle, Duration::from_secs(600));
        let (active, idle) = activity.totals(start + Duration::from_secs(690 + 400));
        assert_eq!(
            (active, idle),
            (Duration::from_secs(90), Duration::from_secs(1000))
        );
    }

    #[test]
    fn memory_logs_capture_events() {
        let (log, events) = UsageLog::memory();
        log.record(|| Event::new("probe").with("n", 1));
        log.set_enabled(false);
        log.record(|| unreachable!());
        assert_eq!(events.lock().unwrap().len(), 1);
        assert!(log.folder().is_none());
    }
}
