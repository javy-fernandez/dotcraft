//! Live `System.Runtime` EventCounters of a running .NET process, read without external tools.
//!
//! [`watch`] talks the .NET Diagnostics IPC protocol over the process's diagnostic port (a Unix
//! domain socket `$TMPDIR/dotnet-diagnostic-{pid}-{disambiguation}-socket` on macOS/Linux, the
//! named pipe `\\.\pipe\dotnet-diagnostic-{pid}` on Windows), starts an EventPipe session with
//! the `System.Runtime` provider (`EventCounterIntervalSec=N`) and parses the NetTrace (V4/V5)
//! stream that follows on the same connection. Each counter arrives as one self-describing
//! `EventCounters` event; the events of one interval are grouped into a [`RuntimeCounters`].
//!
//! Specs: `dotnet/diagnostics` `documentation/design-docs/ipc-protocol.md` and
//! `microsoft/perfview` `src/TraceEvent/EventPipe/NetTraceFormat_v5.md`.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};

/// One interval of `System.Runtime` counters. Fields are `None` when the runtime did not
/// publish that counter (older runtimes publish fewer). `values` keeps every counter by name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuntimeCounters {
    /// `gc-heap-size`, in MB.
    pub gc_heap_mb: Option<f64>,
    /// `gen-0-gc-count`: collections during the interval.
    pub gen0_count: Option<f64>,
    /// `gen-1-gc-count`: collections during the interval.
    pub gen1_count: Option<f64>,
    /// `gen-2-gc-count`: collections during the interval.
    pub gen2_count: Option<f64>,
    /// `time-in-gc`: % of time in GC since the last GC.
    pub time_in_gc_percent: Option<f64>,
    /// `alloc-rate`: bytes allocated during the interval.
    pub alloc_rate_bytes: Option<f64>,
    /// `threadpool-thread-count`.
    pub threadpool_threads: Option<f64>,
    /// `exception-count`: exceptions thrown during the interval.
    pub exceptions_per_interval: Option<f64>,
    /// `working-set`, in MB.
    pub working_set_mb: Option<f64>,
    /// `cpu-usage`, % of all processors.
    pub cpu_percent: Option<f64>,
    /// `gen-0-size`, in bytes.
    pub gen0_size: Option<f64>,
    /// `gen-1-size`, in bytes.
    pub gen1_size: Option<f64>,
    /// `gen-2-size`, in bytes.
    pub gen2_size: Option<f64>,
    /// `loh-size`, in bytes.
    pub loh_size: Option<f64>,
    /// `poh-size`, in bytes.
    pub poh_size: Option<f64>,
    /// Every counter of the interval by its `System.Runtime` name (`Mean` for gauges,
    /// `Increment` for rate counters).
    pub values: BTreeMap<String, f64>,
}

impl RuntimeCounters {
    /// Fills the named fields from the raw counters.
    pub fn from_values(values: BTreeMap<String, f64>) -> Self {
        let g = |n: &str| values.get(n).copied();
        Self {
            gc_heap_mb: g("gc-heap-size"),
            gen0_count: g("gen-0-gc-count"),
            gen1_count: g("gen-1-gc-count"),
            gen2_count: g("gen-2-gc-count"),
            time_in_gc_percent: g("time-in-gc"),
            alloc_rate_bytes: g("alloc-rate"),
            threadpool_threads: g("threadpool-thread-count"),
            exceptions_per_interval: g("exception-count"),
            working_set_mb: g("working-set"),
            cpu_percent: g("cpu-usage"),
            gen0_size: g("gen-0-size"),
            gen1_size: g("gen-1-size"),
            gen2_size: g("gen-2-size"),
            loh_size: g("loh-size"),
            poh_size: g("poh-size"),
            values,
        }
    }
}

/// Connects to the diagnostic port of `pid`, starts an EventPipe session with the
/// `System.Runtime` provider (`EventCounterIntervalSec = interval_secs`) and calls `on_sample`
/// with each complete set of counters (one per interval) until the process exits or `stop` is
/// set; both end with `Ok(())`.
///
/// Blocking; run it on its own thread. Waits up to 10 s for the diagnostic port to appear (so it
/// can be called right after launching the process). Errors when the port cannot be reached, the
/// runtime rejects the session, or the stream is malformed.
pub fn watch(
    pid: u32,
    interval_secs: u32,
    stop: Arc<AtomicBool>,
    mut on_sample: impl FnMut(RuntimeCounters),
) -> Result<()> {
    let interval_secs = interval_secs.max(1);
    let Some(mut conn) = connect_waiting(pid, &stop)? else {
        return Ok(());
    };
    conn.write_all(&ipc_message(
        EVENTPIPE,
        COLLECT_TRACING2,
        &collect_tracing2_payload(interval_secs),
    )?)?;
    let response = read_response(&mut conn).context("starting the EventPipe session")?;
    let session = Cursor::new(&response)
        .u64()
        .context("EventPipe session id")?;
    set_read_timeout(&conn);

    let done = AtomicBool::new(false);
    let result = std::thread::scope(|s| {
        // Ends the session from another connection when `stop` is set, so a blocking read on the
        // stream (Windows pipes have no read timeout) sees the end of the stream.
        s.spawn(|| {
            while !done.load(Ordering::Relaxed) {
                if stop.load(Ordering::Relaxed) {
                    let _ = send_stop(pid, session);
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let mut agg = Aggregator::new(&mut on_sample);
        let r = parse_nettrace(
            StopRead {
                inner: conn,
                stop: &stop,
            },
            &mut agg,
        );
        agg.flush();
        done.store(true, Ordering::Relaxed);
        r
    });
    match result {
        Ok(()) => Ok(()),
        Err(_) if stop.load(Ordering::Relaxed) => Ok(()),
        Err(e) if e.is::<StreamEnded>() => Ok(()),
        Err(e) => Err(e),
    }
}

// --- Diagnostic IPC -------------------------------------------------------------------------

const IPC_MAGIC: &[u8; 14] = b"DOTNET_IPC_V1\0";
const EVENTPIPE: u8 = 0x02;
const STOP_TRACING: u8 = 0x01;
const COLLECT_TRACING2: u8 = 0x03;
const SERVER: u8 = 0xFF;
const SERVER_OK: u8 = 0x00;
const SERVER_ERROR: u8 = 0xFF;

fn ipc_message(command_set: u8, command_id: u8, payload: &[u8]) -> Result<Vec<u8>> {
    let size = u16::try_from(20 + payload.len()).context("IPC payload too large")?;
    let mut m = Vec::with_capacity(size as usize);
    m.extend_from_slice(IPC_MAGIC);
    m.extend_from_slice(&size.to_le_bytes());
    m.push(command_set);
    m.push(command_id);
    m.extend_from_slice(&0u16.to_le_bytes());
    m.extend_from_slice(payload);
    Ok(m)
}

fn put_ipc_string(buf: &mut Vec<u8>, s: &str) {
    let units: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    buf.extend_from_slice(&(units.len() as u32).to_le_bytes());
    for u in units {
        buf.extend_from_slice(&u.to_le_bytes());
    }
}

/// `CollectTracing2`: 10 MB buffer, NetTrace, no rundown, `System.Runtime` with every keyword at
/// Informational level (as `dotnet-counters` does).
fn collect_tracing2_payload(interval_secs: u32) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&10u32.to_le_bytes()); // circularBufferMB
    p.extend_from_slice(&1u32.to_le_bytes()); // format: NetTrace
    p.push(0); // requestRundown
    p.extend_from_slice(&1u32.to_le_bytes()); // provider count
    p.extend_from_slice(&0xFFFF_FFFFu64.to_le_bytes()); // keywords
    p.extend_from_slice(&4u32.to_le_bytes()); // level: Informational
    put_ipc_string(&mut p, "System.Runtime");
    put_ipc_string(&mut p, &format!("EventCounterIntervalSec={interval_secs}"));
    p
}

/// Reads one IPC response and returns its payload, or an error for a Server/Error response.
fn read_response(r: &mut impl Read) -> Result<Vec<u8>> {
    let mut header = [0u8; 20];
    r.read_exact(&mut header)
        .context("reading the IPC response")?;
    if &header[..14] != IPC_MAGIC {
        bail!("not a .NET diagnostic IPC response");
    }
    let size = u16::from_le_bytes([header[14], header[15]]) as usize;
    let payload_len = size
        .checked_sub(20)
        .ok_or_else(|| anyhow!("invalid IPC response size {size}"))?;
    let mut payload = vec![0u8; payload_len];
    r.read_exact(&mut payload)
        .context("reading the IPC response")?;
    match (header[16], header[17]) {
        (SERVER, SERVER_OK) => Ok(payload),
        (SERVER, SERVER_ERROR) => {
            let hr = Cursor::new(&payload).u32().unwrap_or(0);
            bail!("the .NET runtime rejected the request (HRESULT 0x{hr:08X})")
        }
        (set, id) => bail!("unexpected IPC response (command {set:#04x}{id:02x})"),
    }
}

fn send_stop(pid: u32, session: u64) -> Result<()> {
    let mut c = connect(pid)?;
    c.write_all(&ipc_message(
        EVENTPIPE,
        STOP_TRACING,
        &session.to_le_bytes(),
    )?)?;
    read_response(&mut c).map(|_| ())
}

#[cfg(unix)]
type Conn = std::os::unix::net::UnixStream;
#[cfg(windows)]
type Conn = std::fs::File;

/// Retries [`connect`] for up to 10 s; `None` when `stop` was set meanwhile.
fn connect_waiting(pid: u32, stop: &AtomicBool) -> Result<Option<Conn>> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match connect(pid) {
            Ok(c) => return Ok(Some(c)),
            Err(e) if Instant::now() >= deadline => return Err(e),
            Err(_) if stop.load(Ordering::Relaxed) => return Ok(None),
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

/// The diagnostic sockets of `pid`, newest first (a recycled pid can leave stale ones).
#[cfg(unix)]
pub fn diagnostic_sockets(pid: u32) -> Vec<std::path::PathBuf> {
    let prefix = format!("dotnet-diagnostic-{pid}-");
    let mut found: Vec<(std::time::SystemTime, std::path::PathBuf)> =
        std::fs::read_dir(std::env::temp_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                name.starts_with(&prefix) && name.ends_with("-socket")
            })
            .map(|e| {
                (
                    e.metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::UNIX_EPOCH),
                    e.path(),
                )
            })
            .collect();
    found.sort_by_key(|a| std::cmp::Reverse(a.0));
    found.into_iter().map(|(_, p)| p).collect()
}

#[cfg(unix)]
fn connect(pid: u32) -> Result<Conn> {
    let sockets = diagnostic_sockets(pid);
    let mut last = None;
    for path in &sockets {
        match Conn::connect(path) {
            Ok(c) => return Ok(c),
            Err(e) => last = Some(anyhow!("{}: {e}", path.display())),
        }
    }
    Err(last.unwrap_or_else(|| {
        anyhow!(
            "no .NET diagnostic port for process {pid} in {}",
            std::env::temp_dir().display()
        )
    }))
}

#[cfg(windows)]
fn connect(pid: u32) -> Result<Conn> {
    let path = format!(r"\\.\pipe\dotnet-diagnostic-{pid}");
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("no .NET diagnostic port for process {pid} ({path})"))
}

#[cfg(unix)]
fn set_read_timeout(c: &Conn) {
    let _ = c.set_read_timeout(Some(Duration::from_millis(250)));
}

#[cfg(windows)]
fn set_read_timeout(_: &Conn) {}

/// Retries reads that time out, and gives up once `stop` is set.
struct StopRead<'a, R> {
    inner: R,
    stop: &'a AtomicBool,
}

impl<R: Read> Read for StopRead<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return Err(io::Error::other("stopped"));
            }
            match self.inner.read(buf) {
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) => {}
                r => return r,
            }
        }
    }
}

// --- Counter aggregation ----------------------------------------------------------------------

/// Groups the per-counter events of one interval into samples. An interval is complete when a
/// counter name repeats or an EventBlock ends with at least as many counters as the largest
/// interval seen so far (the runtime writes all counters of an interval together).
struct Aggregator<'a> {
    pending: BTreeMap<String, f64>,
    expected: usize,
    on_sample: &'a mut dyn FnMut(RuntimeCounters),
}

impl<'a> Aggregator<'a> {
    fn new(on_sample: &'a mut dyn FnMut(RuntimeCounters)) -> Self {
        Self {
            pending: BTreeMap::new(),
            expected: 0,
            on_sample,
        }
    }

    fn counter(&mut self, name: String, value: f64) {
        if self.pending.contains_key(&name) {
            self.flush();
        }
        self.pending.insert(name, value);
    }

    fn block_end(&mut self) {
        if self.pending.len() >= self.expected {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        self.expected = self.expected.max(self.pending.len());
        (self.on_sample)(RuntimeCounters::from_values(std::mem::take(
            &mut self.pending,
        )));
    }
}

/// `(name, value)` of an `EventCounters` event: `Increment` for rate counters, `Mean` for gauges.
fn counter_value(fields: &[Field], payload: &[u8]) -> Result<Option<(String, f64)>> {
    let mut c = Cursor::new(payload);
    let top = decode_fields(&mut c, fields, 0)?;
    // The runtime wraps the counter in `{ "": { Payload: { Name, Mean, ... } } }`.
    fn find(fields: &[(String, Value)], depth: usize) -> Option<&[(String, Value)]> {
        if fields
            .iter()
            .any(|(k, v)| k == "Name" && matches!(v, Value::Str(_)))
        {
            return Some(fields);
        }
        if depth >= MAX_DEPTH {
            return None;
        }
        fields.iter().find_map(|(_, v)| match v {
            Value::Obj(o) => find(o, depth + 1),
            _ => None,
        })
    }
    let Some(obj) = find(&top, 0) else {
        return Ok(None);
    };
    let get = |n: &str| obj.iter().find(|(k, _)| k == n).map(|(_, v)| v);
    let Some(Value::Str(name)) = get("Name") else {
        return Ok(None);
    };
    let num = |n: &str| match get(n) {
        Some(Value::Num(v)) => Some(*v),
        _ => None,
    };
    let is_rate = matches!(get("CounterType"), Some(Value::Str(t)) if t == "Sum");
    let value = if is_rate {
        num("Increment").or(num("Mean"))
    } else {
        num("Mean").or(num("Increment"))
    };
    Ok(value.map(|v| (name.clone(), v)))
}

// --- NetTrace stream --------------------------------------------------------------------------

/// The stream ended (the process exited or the session was stopped).
#[derive(Debug)]
struct StreamEnded;

impl std::fmt::Display for StreamEnded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the EventPipe stream ended")
    }
}

impl std::error::Error for StreamEnded {}

/// Largest block accepted (the runtime writes blocks of at most a few hundred KB).
const MAX_BLOCK: usize = 64 << 20;
const TAG_NULL: u8 = 1;
const TAG_BEGIN_PRIVATE_OBJECT: u8 = 5;
const TAG_END_OBJECT: u8 = 6;

struct Stream<R> {
    r: R,
    pos: u64,
}

impl<R: Read> Stream<R> {
    fn bytes(&mut self, n: usize) -> Result<Vec<u8>> {
        let mut b = vec![0u8; n];
        match self.r.read_exact(&mut b) {
            Ok(()) => {
                self.pos += n as u64;
                Ok(b)
            }
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Err(StreamEnded.into()),
            Err(e) => Err(e.into()),
        }
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    fn i32(&mut self) -> Result<i32> {
        let b = self.bytes(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn len(&mut self, max: usize, what: &str) -> Result<usize> {
        let n = self.i32()?;
        usize::try_from(n)
            .ok()
            .filter(|&n| n <= max)
            .ok_or_else(|| anyhow!("invalid {what} size {n}"))
    }

    fn expect_tag(&mut self, tag: u8) -> Result<()> {
        let t = self.u8()?;
        if t != tag {
            bail!(
                "malformed nettrace stream: tag {t} where {tag} was expected at offset {}",
                self.pos - 1
            );
        }
        Ok(())
    }

    /// Reads the serialized type of an object (after its BeginPrivateObject tag) and returns its name.
    fn object_type(&mut self) -> Result<String> {
        self.expect_tag(TAG_BEGIN_PRIVATE_OBJECT)?;
        self.expect_tag(TAG_NULL)?;
        let _version = self.i32()?;
        let _min_reader_version = self.i32()?;
        let len = self.len(256, "type name")?;
        let name = String::from_utf8(self.bytes(len)?)
            .map_err(|_| anyhow!("invalid nettrace type name"))?;
        self.expect_tag(TAG_END_OBJECT)?;
        Ok(name)
    }
}

/// Parses a NetTrace V4/V5 stream until its end tag, feeding `EventCounters` events to `agg`.
fn parse_nettrace(r: impl Read, agg: &mut Aggregator) -> Result<()> {
    let mut s = Stream { r, pos: 0 };
    if s.bytes(8)? != b"Nettrace" {
        bail!("not a nettrace stream");
    }
    let header = s.bytes(24)?;
    if header[..4] != 20i32.to_le_bytes() || &header[4..] != b"!FastSerialization.1" {
        bail!("unsupported nettrace version (only V4/V5 are supported)");
    }
    s.expect_tag(TAG_BEGIN_PRIVATE_OBJECT)?;
    let name = s.object_type()?;
    if name != "Trace" {
        bail!("nettrace stream starts with {name} instead of Trace");
    }
    // SyncTimeUTC (8 shorts), SyncTimeQPC, QPCFrequency, PointerSize, ProcessId,
    // NumberOfProcessors, ExpectedCPUSamplingRate.
    s.bytes(16 + 8 + 8 + 4 * 4)?;
    s.expect_tag(TAG_END_OBJECT)?;

    let mut metadata: HashMap<u32, EventMeta> = HashMap::new();
    loop {
        match s.u8()? {
            TAG_NULL => return Ok(()),
            TAG_BEGIN_PRIVATE_OBJECT => {}
            t => bail!(
                "malformed nettrace stream: unexpected tag {t} at offset {}",
                s.pos - 1
            ),
        }
        let kind = s.object_type()?;
        if !matches!(
            kind.as_str(),
            "EventBlock" | "MetadataBlock" | "StackBlock" | "SPBlock"
        ) {
            bail!("unsupported nettrace object {kind}");
        }
        let size = s.len(MAX_BLOCK, "block")?;
        let pad = ((4 - s.pos % 4) % 4) as usize;
        s.bytes(pad)?;
        let block = s.bytes(size)?;
        s.expect_tag(TAG_END_OBJECT)?;
        match kind.as_str() {
            "MetadataBlock" => read_events(&block, |_, payload| {
                if let Some(meta) = parse_metadata(payload)? {
                    metadata.insert(meta.id, meta);
                }
                Ok(())
            })?,
            "EventBlock" => {
                read_events(&block, |id, payload| {
                    let Some(meta) = metadata.get(&id) else {
                        return Ok(());
                    };
                    if meta.event_name != "EventCounters" {
                        return Ok(());
                    }
                    let Some(fields) = &meta.fields else {
                        return Ok(());
                    };
                    if let Some((name, v)) = counter_value(fields, payload)? {
                        agg.counter(name, v);
                    }
                    Ok(())
                })?;
                agg.block_end();
            }
            _ => {} // stacks and sequence points are not needed
        }
    }
}

/// Header fields carried between compressed event blobs.
#[derive(Default)]
struct BlobHeader {
    metadata_id: u32,
    sequence: u32,
    capture_thread: u64,
    processor: u32,
    thread: u64,
    stack: u32,
    timestamp: u64,
    payload_size: u32,
}

/// Calls `f(metadata_id, payload)` for each event blob of an EventBlock or MetadataBlock.
fn read_events(block: &[u8], mut f: impl FnMut(u32, &[u8]) -> Result<()>) -> Result<()> {
    let mut c = Cursor::new(block);
    let header_size = c.u16()? as usize;
    let flags = c.u16()?;
    if header_size < 4 {
        bail!("invalid event block header size {header_size}");
    }
    c.take(header_size - 4)?;
    let compressed = flags & 1 != 0;
    let mut h = BlobHeader::default();
    while !c.is_empty() {
        if compressed {
            let flags = c.u8()?;
            if flags & 1 != 0 {
                h.metadata_id = c.var_u32()?;
            }
            if flags & 2 != 0 {
                h.sequence = h.sequence.wrapping_add(c.var_u32()?);
                h.capture_thread = c.var_u64()?;
                h.processor = c.var_u32()?;
            }
            if h.metadata_id != 0 {
                h.sequence = h.sequence.wrapping_add(1);
            }
            if flags & 4 != 0 {
                h.thread = c.var_u64()?;
            }
            if flags & 8 != 0 {
                h.stack = c.var_u32()?;
            }
            h.timestamp = h.timestamp.wrapping_add(c.var_u64()?);
            if flags & 16 != 0 {
                c.take(16)?; // ActivityId
            }
            if flags & 32 != 0 {
                c.take(16)?; // RelatedActivityId
            }
            if flags & 128 != 0 {
                h.payload_size = c.var_u32()?;
            }
            let payload = c.take(h.payload_size as usize)?;
            f(h.metadata_id & 0x7FFF_FFFF, payload)?;
        } else {
            let _event_size = c.i32()?;
            let metadata_id = c.u32()? & 0x7FFF_FFFF;
            // Sequence, ThreadId, CaptureThreadId, ProcessorNumber, StackId, TimeStamp,
            // ActivityID, RelatedActivityID.
            c.take(4 + 8 + 8 + 4 + 4 + 8 + 16 + 16)?;
            let size = c.u32()? as usize;
            let payload = c.take(size)?;
            f(metadata_id, payload)?;
            // Blobs are aligned to 4 bytes; blocks start aligned, so the block offset is enough.
            let pad = (4 - c.pos % 4) % 4;
            c.take(pad.min(c.remaining()))?;
        }
    }
    Ok(())
}

/// One event type of the trace.
#[derive(Debug, Clone, PartialEq)]
struct EventMeta {
    id: u32,
    provider: String,
    event_name: String,
    /// `None` when the field layout uses something this parser does not understand.
    fields: Option<Vec<Field>>,
}

#[derive(Debug, Clone, PartialEq)]
struct Field {
    name: String,
    ty: Ty,
}

#[derive(Debug, Clone, PartialEq)]
enum Ty {
    /// A primitive of a fixed size, by its EventPipe type code.
    Fixed(u8, usize),
    /// UTF-16, null terminated.
    Str,
    Obj(Vec<Field>),
    /// `u16` count followed by the elements.
    Arr(Box<Ty>),
}

const MAX_DEPTH: usize = 16;
const MAX_FIELDS: usize = 4096;

/// Parses a metadata record (the payload of a MetadataBlock blob). `Ok(None)` for an empty record.
fn parse_metadata(payload: &[u8]) -> Result<Option<EventMeta>> {
    let mut c = Cursor::new(payload);
    if c.is_empty() {
        return Ok(None);
    }
    let id = c.u32()?;
    let provider = c.utf16z()?;
    let _event_id = c.i32()?;
    let event_name = c.utf16z()?;
    let _keywords = c.u64()?;
    let _version = c.i32()?;
    let _level = c.i32()?;
    let fields = parse_metadata_fields(&mut c).ok();
    Ok(Some(EventMeta {
        id,
        provider,
        event_name,
        fields,
    }))
}

/// V1 field list, then the V5 tags (a V2Params tag replaces the V1 fields).
fn parse_metadata_fields(c: &mut Cursor) -> Result<Vec<Field>> {
    if c.is_empty() {
        return Ok(Vec::new());
    }
    let mut fields = parse_fields(c, 0, false)?;
    while !c.is_empty() {
        let size = c.u32()? as usize;
        let kind = c.u8()?;
        let data = c.take(size)?;
        if kind == 2 {
            fields = parse_fields(&mut Cursor::new(data), 0, true)?;
        }
    }
    Ok(fields)
}

fn parse_fields(c: &mut Cursor, depth: usize, v2: bool) -> Result<Vec<Field>> {
    if depth > MAX_DEPTH {
        bail!("event metadata nested too deeply");
    }
    let count = c.u32()? as usize;
    if count > MAX_FIELDS {
        bail!("too many event fields ({count})");
    }
    let mut fields = Vec::with_capacity(count);
    for _ in 0..count {
        if v2 {
            let size = c.u32()? as usize;
            let mut f = Cursor::new(
                c.take(
                    size.checked_sub(4)
                        .ok_or_else(|| anyhow!("invalid field size {size}"))?,
                )?,
            );
            let name = f.utf16z()?;
            let code = f.u32()?;
            let ty = parse_type(&mut f, code, depth, true)?;
            fields.push(Field { name, ty });
        } else {
            let code = c.u32()?;
            let ty = parse_type(c, code, depth, false)?;
            let name = c.utf16z()?;
            fields.push(Field { name, ty });
        }
    }
    Ok(fields)
}

fn parse_type(c: &mut Cursor, code: u32, depth: usize, v2: bool) -> Result<Ty> {
    let fixed = |size| Ok(Ty::Fixed(code as u8, size));
    match code {
        1 => Ok(Ty::Obj(parse_fields(c, depth + 1, v2)?)),
        3 | 9 | 10 | 13 => fixed(4),
        4 | 7 | 8 => fixed(2),
        5 | 6 => fixed(1),
        11 | 12 | 14 | 16 => fixed(8),
        15 | 17 => fixed(16),
        18 => Ok(Ty::Str),
        19 if v2 => {
            if depth > MAX_DEPTH {
                bail!("event metadata nested too deeply");
            }
            let element = c.u32()?;
            Ok(Ty::Arr(Box::new(parse_type(c, element, depth + 1, v2)?)))
        }
        _ => bail!("unsupported event field type code {code}"),
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Value {
    Num(f64),
    Str(String),
    Obj(Vec<(String, Value)>),
    Arr(Vec<Value>),
    Other,
}

fn decode_fields(c: &mut Cursor, fields: &[Field], depth: usize) -> Result<Vec<(String, Value)>> {
    fields
        .iter()
        .map(|f| Ok((f.name.clone(), decode(c, &f.ty, depth)?)))
        .collect()
}

fn decode(c: &mut Cursor, ty: &Ty, depth: usize) -> Result<Value> {
    if depth > MAX_DEPTH {
        bail!("event payload nested too deeply");
    }
    Ok(match ty {
        Ty::Str => Value::Str(c.utf16z()?),
        Ty::Obj(fields) => Value::Obj(decode_fields(c, fields, depth + 1)?),
        Ty::Arr(element) => {
            let n = c.u16()?;
            Value::Arr(
                (0..n)
                    .map(|_| decode(c, element, depth + 1))
                    .collect::<Result<_>>()?,
            )
        }
        Ty::Fixed(code, size) => {
            let b = c.take(*size)?;
            let n = |i: usize| -> [u8; 8] {
                let mut a = [0u8; 8];
                a[..i].copy_from_slice(&b[..i]);
                a
            };
            match code {
                3 | 9 => Value::Num(i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64),
                5 => Value::Num(b[0] as i8 as f64),
                6 => Value::Num(b[0] as f64),
                7 => Value::Num(i16::from_le_bytes([b[0], b[1]]) as f64),
                8 => Value::Num(u16::from_le_bytes([b[0], b[1]]) as f64),
                10 => Value::Num(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64),
                11 => Value::Num(i64::from_le_bytes(n(8)) as f64),
                12 => Value::Num(u64::from_le_bytes(n(8)) as f64),
                13 => Value::Num(f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64),
                14 => Value::Num(f64::from_le_bytes(n(8))),
                _ => Value::Other,
            }
        }
    })
}

/// Bounds-checked little-endian reader over a byte slice.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.remaining() {
            bail!("truncated nettrace data");
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut a = [0u8; N];
        a.copy_from_slice(self.take(N)?);
        Ok(a)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn var_u64(&mut self) -> Result<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.u8()?;
            v |= u64::from(b & 0x7F) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        bail!("invalid varint in nettrace data")
    }

    fn var_u32(&mut self) -> Result<u32> {
        u32::try_from(self.var_u64()?).map_err(|_| anyhow!("varint out of range in nettrace data"))
    }

    /// UTF-16LE up to (and consuming) a null code unit.
    fn utf16z(&mut self) -> Result<String> {
        let mut units = Vec::new();
        loop {
            match self.u16()? {
                0 => break,
                u => units.push(u),
            }
        }
        Ok(String::from_utf16_lossy(&units))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16z(out: &mut Vec<u8>, s: &str) {
        for u in s.encode_utf16().chain(std::iter::once(0)) {
            out.extend_from_slice(&u.to_le_bytes());
        }
    }

    fn i32le(out: &mut Vec<u8>, v: i32) {
        out.extend_from_slice(&v.to_le_bytes());
    }

    fn varint(out: &mut Vec<u8>, mut v: u64) {
        loop {
            let b = (v & 0x7F) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                return;
            }
            out.push(b | 0x80);
        }
    }

    fn object_type(out: &mut Vec<u8>, name: &str) {
        out.extend_from_slice(&[TAG_BEGIN_PRIVATE_OBJECT, TAG_BEGIN_PRIVATE_OBJECT, TAG_NULL]);
        i32le(out, 2);
        i32le(out, 2);
        i32le(out, name.len() as i32);
        out.extend_from_slice(name.as_bytes());
        out.push(TAG_END_OBJECT);
    }

    fn block(out: &mut Vec<u8>, name: &str, data: &[u8]) {
        object_type(out, name);
        i32le(out, data.len() as i32);
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
        out.extend_from_slice(data);
        out.push(TAG_END_OBJECT);
    }

    /// Block header (20 bytes, compressed) followed by compressed blobs `(metadata_id, payload)`.
    fn compressed_blobs(blobs: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&20u16.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&[0u8; 16]);
        for (id, payload) in blobs {
            d.push(1 | 128);
            varint(&mut d, *id as u64);
            varint(&mut d, 10); // timestamp delta
            varint(&mut d, payload.len() as u64);
            d.extend_from_slice(payload);
        }
        d
    }

    fn metadata_header(id: u32, provider: &str, name: &str) -> Vec<u8> {
        let mut m = Vec::new();
        i32le(&mut m, id as i32);
        utf16z(&mut m, provider);
        i32le(&mut m, 0);
        utf16z(&mut m, name);
        m.extend_from_slice(&0u64.to_le_bytes());
        i32le(&mut m, 0);
        i32le(&mut m, 4);
        m
    }

    /// `EventCounters` with a V1 nested `Payload { Name, Mean, CounterType }`.
    fn mean_metadata(id: u32) -> Vec<u8> {
        let mut m = metadata_header(id, "System.Runtime", "EventCounters");
        i32le(&mut m, 1);
        i32le(&mut m, 1); // Object
        i32le(&mut m, 3);
        for (code, name) in [(18, "Name"), (14, "Mean"), (18, "CounterType")] {
            i32le(&mut m, code);
            utf16z(&mut m, name);
        }
        utf16z(&mut m, "Payload");
        m
    }

    /// `EventCounters` described by a V2Params tag (empty V1 list), with an array field.
    fn increment_metadata_v2(id: u32) -> Vec<u8> {
        let field = |name: &str, body: &[u8]| {
            let mut f = Vec::new();
            utf16z(&mut f, name);
            f.extend_from_slice(body);
            let mut out = Vec::new();
            i32le(&mut out, (f.len() + 4) as i32);
            out.extend_from_slice(&f);
            out
        };
        let mut inner = Vec::new();
        i32le(&mut inner, 4);
        inner.extend(field("Name", &18i32.to_le_bytes()));
        inner.extend(field(
            "Tags",
            &[19i32.to_le_bytes(), 9i32.to_le_bytes()].concat(),
        ));
        inner.extend(field("Increment", &14i32.to_le_bytes()));
        inner.extend(field("CounterType", &18i32.to_le_bytes()));
        let mut tag = Vec::new();
        i32le(&mut tag, 1);
        tag.extend(field(
            "Payload",
            &[1i32.to_le_bytes().as_slice(), &inner].concat(),
        ));

        let mut m = metadata_header(id, "System.Runtime", "EventCounters");
        i32le(&mut m, 0); // no V1 fields
        i32le(&mut m, 1); // OpCode tag
        m.push(1);
        m.push(0);
        i32le(&mut m, tag.len() as i32);
        m.push(2); // V2Params
        m.extend_from_slice(&tag);
        m
    }

    fn mean_event(name: &str, v: f64) -> Vec<u8> {
        let mut p = Vec::new();
        utf16z(&mut p, name);
        p.extend_from_slice(&v.to_le_bytes());
        utf16z(&mut p, "Mean");
        p
    }

    fn increment_event(name: &str, v: f64) -> Vec<u8> {
        let mut p = Vec::new();
        utf16z(&mut p, name);
        p.extend_from_slice(&2u16.to_le_bytes());
        i32le(&mut p, 7);
        i32le(&mut p, 8);
        p.extend_from_slice(&v.to_le_bytes());
        utf16z(&mut p, "Sum");
        p
    }

    fn stream(blocks: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut s = b"Nettrace".to_vec();
        i32le(&mut s, 20);
        s.extend_from_slice(b"!FastSerialization.1");
        object_type(&mut s, "Trace");
        s.extend_from_slice(&[0u8; 48]);
        s.push(TAG_END_OBJECT);
        for (name, data) in blocks {
            block(&mut s, name, data);
        }
        s.push(TAG_NULL);
        s
    }

    fn run(bytes: &[u8]) -> (Result<()>, Vec<RuntimeCounters>) {
        let mut samples = Vec::new();
        let mut cb = |c: RuntimeCounters| samples.push(c);
        let mut agg = Aggregator::new(&mut cb);
        let r = parse_nettrace(bytes, &mut agg);
        agg.flush();
        (r, samples)
    }

    fn sample_stream() -> Vec<u8> {
        let meta = compressed_blobs(&[(0, mean_metadata(1)), (0, increment_metadata_v2(2))]);
        let round = |heap: f64, gen0: f64| {
            compressed_blobs(&[
                (1, mean_event("gc-heap-size", heap)),
                (2, increment_event("gen-0-gc-count", gen0)),
                (1, mean_event("cpu-usage", 1.5)),
            ])
        };
        stream(&[
            ("MetadataBlock", meta),
            ("EventBlock", round(12.5, 3.0)),
            ("SPBlock", vec![0; 12]),
            ("EventBlock", round(13.0, 4.0)),
        ])
    }

    #[test]
    fn varints() {
        let mut b = Vec::new();
        varint(&mut b, 300);
        varint(&mut b, u64::MAX);
        let mut c = Cursor::new(&b);
        assert_eq!(c.var_u32().unwrap(), 300);
        assert_eq!(c.var_u64().unwrap(), u64::MAX);
        assert!(Cursor::new(&[0x80; 11]).var_u64().is_err());
        assert!(Cursor::new(&[0x80]).var_u64().is_err());
    }

    #[test]
    fn parses_v1_and_v2_metadata() {
        let m = parse_metadata(&mean_metadata(1)).unwrap().unwrap();
        assert_eq!(
            (m.id, m.provider.as_str(), m.event_name.as_str()),
            (1, "System.Runtime", "EventCounters")
        );
        let Some(fields) = m.fields else { panic!() };
        assert_eq!(fields[0].name, "Payload");
        let Ty::Obj(inner) = &fields[0].ty else {
            panic!()
        };
        assert_eq!(
            inner.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
            ["Name", "Mean", "CounterType"]
        );

        let m = parse_metadata(&increment_metadata_v2(2)).unwrap().unwrap();
        let fields = m.fields.unwrap();
        let Ty::Obj(inner) = &fields[0].ty else {
            panic!()
        };
        assert_eq!(inner[1].ty, Ty::Arr(Box::new(Ty::Fixed(9, 4))));
    }

    #[test]
    fn unknown_field_types_drop_only_the_fields() {
        let mut m = metadata_header(5, "X", "Y");
        i32le(&mut m, 1);
        i32le(&mut m, 99);
        utf16z(&mut m, "Weird");
        let meta = parse_metadata(&m).unwrap().unwrap();
        assert_eq!(meta.fields, None);
    }

    #[test]
    fn reads_counter_values() {
        let fields = parse_metadata(&mean_metadata(1))
            .unwrap()
            .unwrap()
            .fields
            .unwrap();
        assert_eq!(
            counter_value(&fields, &mean_event("working-set", 42.0)).unwrap(),
            Some(("working-set".into(), 42.0))
        );
        let fields = parse_metadata(&increment_metadata_v2(2))
            .unwrap()
            .unwrap()
            .fields
            .unwrap();
        assert_eq!(
            counter_value(&fields, &increment_event("alloc-rate", 1024.0)).unwrap(),
            Some(("alloc-rate".into(), 1024.0))
        );
        assert!(counter_value(&fields, &increment_event("alloc-rate", 1.0)[..10]).is_err());
    }

    #[test]
    fn groups_counters_per_interval() {
        let (r, samples) = run(&sample_stream());
        r.unwrap();
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].gc_heap_mb, Some(12.5));
        assert_eq!(samples[0].gen0_count, Some(3.0));
        assert_eq!(samples[0].cpu_percent, Some(1.5));
        assert_eq!(samples[0].working_set_mb, None);
        assert_eq!(samples[1].gc_heap_mb, Some(13.0));
        assert_eq!(samples[1].values.len(), 3);
    }

    #[test]
    fn repeated_counter_starts_a_new_interval() {
        let meta = compressed_blobs(&[(0, mean_metadata(1))]);
        let events = compressed_blobs(&[
            (1, mean_event("gc-heap-size", 1.0)),
            (1, mean_event("cpu-usage", 2.0)),
            (1, mean_event("gc-heap-size", 3.0)),
        ]);
        let (r, samples) = run(&stream(&[("MetadataBlock", meta), ("EventBlock", events)]));
        r.unwrap();
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[1].gc_heap_mb, Some(3.0));
    }

    #[test]
    fn malformed_streams_error_without_panicking() {
        let good = sample_stream();
        // Every truncation either ends cleanly (stream ended) or errors; none panics.
        for n in 0..good.len() {
            let _ = run(&good[..n]);
        }
        // Flipping each byte must not panic either.
        for i in 0..good.len() {
            let mut bad = good.clone();
            bad[i] ^= 0xFF;
            let _ = run(&bad);
        }
        let (r, _) = run(b"NotTrace........................");
        assert!(r.is_err());
        let (r, _) = run(&good[..good.len() - 1]);
        assert!(r.unwrap_err().is::<StreamEnded>());
    }

    #[test]
    fn ipc_messages() {
        let m = ipc_message(EVENTPIPE, COLLECT_TRACING2, &collect_tracing2_payload(1)).unwrap();
        assert_eq!(&m[..14], IPC_MAGIC);
        assert_eq!(u16::from_le_bytes([m[14], m[15]]) as usize, m.len());
        assert_eq!((m[16], m[17]), (2, 3));

        let mut ok = ipc_message(SERVER, SERVER_OK, &7u64.to_le_bytes()).unwrap();
        assert_eq!(
            read_response(&mut ok.as_slice()).unwrap(),
            7u64.to_le_bytes()
        );
        let err = ipc_message(SERVER, SERVER_ERROR, &0x8013_1384u32.to_le_bytes()).unwrap();
        assert!(
            read_response(&mut err.as_slice())
                .unwrap_err()
                .to_string()
                .contains("0x80131384")
        );
        ok[14] = 3; // size below the header
        assert!(read_response(&mut ok.as_slice()).is_err());
    }
}
