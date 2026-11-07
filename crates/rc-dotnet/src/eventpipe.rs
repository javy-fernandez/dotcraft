//! Shared .NET Diagnostics IPC + EventPipe plumbing: the diagnostic port transport (a Unix domain
//! socket `$TMPDIR/dotnet-diagnostic-{pid}-{disambiguation}-socket` on macOS/Linux, the named pipe
//! `\\.\pipe\dotnet-diagnostic-{pid}` on Windows), `CollectTracing2`/`StopTracing` and a NetTrace
//! (V4/V5) stream parser that feeds events, stacks and sequence points to a [`Sink`].
//!
//! Used by `counters` (EventCounters), `gcdump` (heap snapshots) and `cpu_profile` (sampling).
//!
//! Specs: `dotnet/diagnostics` `documentation/design-docs/ipc-protocol.md` and
//! `microsoft/perfview` `src/TraceEvent/EventPipe/NetTraceFormat_v5.md`.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};

// --- Diagnostic IPC -------------------------------------------------------------------------

pub(crate) const IPC_MAGIC: &[u8; 14] = b"DOTNET_IPC_V1\0";
pub(crate) const EVENTPIPE: u8 = 0x02;
pub(crate) const STOP_TRACING: u8 = 0x01;
pub(crate) const COLLECT_TRACING2: u8 = 0x03;
pub(crate) const SERVER: u8 = 0xFF;
pub(crate) const SERVER_OK: u8 = 0x00;
pub(crate) const SERVER_ERROR: u8 = 0xFF;

pub(crate) fn ipc_message(command_set: u8, command_id: u8, payload: &[u8]) -> Result<Vec<u8>> {
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

/// An IPC string: UTF-16 code unit count (null included) and the units; `None` is a 0 count.
fn put_ipc_string(buf: &mut Vec<u8>, s: Option<&str>) {
    let Some(s) = s else {
        buf.extend_from_slice(&0u32.to_le_bytes());
        return;
    };
    let units: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    buf.extend_from_slice(&(units.len() as u32).to_le_bytes());
    for u in units {
        buf.extend_from_slice(&u.to_le_bytes());
    }
}

/// One EventPipe provider of a session.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Provider<'a> {
    pub name: &'a str,
    pub keywords: u64,
    /// EventLevel: 4 Informational, 5 Verbose.
    pub level: u32,
    /// `key=value;key=value` filter data.
    pub args: Option<&'a str>,
}

/// `CollectTracing2` payload: NetTrace format, `buffer_mb` circular buffer.
pub(crate) fn collect_tracing2_payload(
    buffer_mb: u32,
    rundown: bool,
    providers: &[Provider],
) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&buffer_mb.to_le_bytes()); // circularBufferMB
    p.extend_from_slice(&1u32.to_le_bytes()); // format: NetTrace
    p.push(u8::from(rundown)); // requestRundown
    p.extend_from_slice(&(providers.len() as u32).to_le_bytes());
    for pr in providers {
        p.extend_from_slice(&pr.keywords.to_le_bytes());
        p.extend_from_slice(&pr.level.to_le_bytes());
        put_ipc_string(&mut p, Some(pr.name));
        put_ipc_string(&mut p, pr.args);
    }
    p
}

/// Reads one IPC response and returns its payload, or an error for a Server/Error response.
pub(crate) fn read_response(r: &mut impl Read) -> Result<Vec<u8>> {
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

pub(crate) fn send_stop(pid: u32, session: u64) -> Result<()> {
    let mut c = connect(pid)?;
    c.write_all(&ipc_message(
        EVENTPIPE,
        STOP_TRACING,
        &session.to_le_bytes(),
    )?)?;
    read_response(&mut c).map(|_| ())
}

/// Sends `CollectTracing2` on `conn` and returns the session id; the NetTrace stream follows on
/// the same connection.
pub(crate) fn start_session(conn: &mut Conn, payload: &[u8]) -> Result<u64> {
    conn.write_all(&ipc_message(EVENTPIPE, COLLECT_TRACING2, payload)?)?;
    let response = read_response(conn).context("starting the EventPipe session")?;
    Cursor::new(&response).u64().context("EventPipe session id")
}

#[cfg(unix)]
pub(crate) type Conn = std::os::unix::net::UnixStream;
#[cfg(windows)]
pub(crate) type Conn = std::fs::File;

/// Retries [`connect`] for up to 10 s; `None` when `stop` was set meanwhile.
pub(crate) fn connect_waiting(pid: u32, stop: &AtomicBool) -> Result<Option<Conn>> {
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
pub(crate) fn connect(pid: u32) -> Result<Conn> {
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
pub(crate) fn connect(pid: u32) -> Result<Conn> {
    let path = format!(r"\\.\pipe\dotnet-diagnostic-{pid}");
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("no .NET diagnostic port for process {pid} ({path})"))
}

#[cfg(unix)]
pub(crate) fn set_read_timeout(c: &Conn) {
    let _ = c.set_read_timeout(Some(Duration::from_millis(250)));
}

#[cfg(windows)]
pub(crate) fn set_read_timeout(_: &Conn) {}

/// Retries reads that time out, and gives up once `stop` is set.
pub(crate) struct StopRead<'a, R> {
    pub inner: R,
    pub stop: &'a AtomicBool,
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

/// Runs one EventPipe session on `pid` and feeds its stream to `sink` until the stream ends.
///
/// The session is ended gracefully (`StopTracing`, then the rest of the stream, rundown included,
/// is read) once `stop` or `end` is set or `until` passes; the drain after that is bounded by
/// `grace`. Returns `Ok(true)` when the stream ended normally (also when the process exited),
/// `Ok(false)` when `stop` was set before the port was reached or the drain was cut short.
pub(crate) fn run_session(
    pid: u32,
    payload: &[u8],
    stop: &AtomicBool,
    end: &AtomicBool,
    until: Option<Instant>,
    grace: Duration,
    sink: &mut dyn Sink,
) -> Result<bool> {
    let Some(mut conn) = connect_waiting(pid, stop)? else {
        return Ok(false);
    };
    let session = start_session(&mut conn, payload)?;
    set_read_timeout(&conn);

    let done = AtomicBool::new(false);
    let abort = AtomicBool::new(false);
    let result = std::thread::scope(|s| {
        s.spawn(|| {
            let mut stopped_at: Option<Instant> = None;
            while !done.load(Ordering::Relaxed) {
                let now = Instant::now();
                match stopped_at {
                    None if stop.load(Ordering::Relaxed)
                        || end.load(Ordering::Relaxed)
                        || until.is_some_and(|u| now >= u) =>
                    {
                        let _ = send_stop(pid, session);
                        stopped_at = Some(Instant::now());
                    }
                    Some(t) if now.duration_since(t) > grace => {
                        abort.store(true, Ordering::Relaxed);
                        return;
                    }
                    _ => {}
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let r = parse_nettrace(
            StopRead {
                inner: conn,
                stop: &abort,
            },
            sink,
        );
        done.store(true, Ordering::Relaxed);
        r
    });
    match result {
        Ok(()) => Ok(true),
        Err(e) if e.is::<StreamEnded>() => Ok(true),
        Err(_) if abort.load(Ordering::Relaxed) => Ok(false),
        Err(e) => Err(e),
    }
}

// --- NetTrace stream --------------------------------------------------------------------------

/// The stream ended (the process exited or the session was stopped).
#[derive(Debug)]
pub(crate) struct StreamEnded;

impl std::fmt::Display for StreamEnded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the EventPipe stream ended")
    }
}

impl std::error::Error for StreamEnded {}

/// Receives the contents of a NetTrace stream.
pub(crate) trait Sink {
    /// The trace header: the pointer size of the traced process (4 or 8).
    fn trace(&mut self, _pointer_size: usize) {}
    /// One event whose metadata is known.
    fn event(&mut self, meta: &EventMeta, header: &EventHeader, payload: &[u8]) -> Result<()>;
    /// One stack of a StackBlock: instruction pointers, leaf first.
    fn stack(&mut self, _id: u32, _ips: &[u64]) {}
    /// End of an EventBlock.
    fn block_end(&mut self) {}
}

/// Per-event header fields a sink may need.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct EventHeader {
    pub thread: u64,
    /// 0 when the event has no stack.
    pub stack: u32,
    pub timestamp: u64,
}

/// Largest block accepted (the runtime writes blocks of at most a few hundred KB).
const MAX_BLOCK: usize = 64 << 20;
pub(crate) const TAG_NULL: u8 = 1;
pub(crate) const TAG_BEGIN_PRIVATE_OBJECT: u8 = 5;
pub(crate) const TAG_END_OBJECT: u8 = 6;

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

/// Parses a NetTrace V4/V5 stream until its end tag, feeding it to `sink`.
pub(crate) fn parse_nettrace(r: impl Read, sink: &mut dyn Sink) -> Result<()> {
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
    let trace = s.bytes(16 + 8 + 8 + 4 * 4)?;
    let pointer_size = match i32::from_le_bytes([trace[32], trace[33], trace[34], trace[35]]) {
        4 => 4,
        _ => 8,
    };
    sink.trace(pointer_size);
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
                read_events(&block, |h, payload| {
                    let Some(meta) = metadata.get(&h.metadata_id) else {
                        return Ok(());
                    };
                    let header = EventHeader {
                        thread: h.thread,
                        stack: h.stack,
                        timestamp: h.timestamp,
                    };
                    sink.event(meta, &header, payload)
                })?;
                sink.block_end();
            }
            "StackBlock" => read_stacks(&block, pointer_size, sink)?,
            _ => {} // sequence points: nothing to do
        }
    }
}

/// `FirstId`, `Count`, then `Count` stacks of `Size` bytes of instruction pointers.
fn read_stacks(block: &[u8], pointer_size: usize, sink: &mut dyn Sink) -> Result<()> {
    let mut c = Cursor::new(block);
    let first = c.u32()?;
    let count = c.u32()?;
    let mut ips = Vec::new();
    for i in 0..count {
        let size = c.u32()? as usize;
        let data = c.take(size)?;
        ips.clear();
        ips.extend(data.chunks_exact(pointer_size).map(|ip| {
            let mut a = [0u8; 8];
            a[..pointer_size].copy_from_slice(ip);
            u64::from_le_bytes(a)
        }));
        sink.stack(first.wrapping_add(i), &ips);
    }
    Ok(())
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

/// Calls `f(header, payload)` for each event blob of an EventBlock or MetadataBlock.
fn read_events(block: &[u8], mut f: impl FnMut(&BlobHeader, &[u8]) -> Result<()>) -> Result<()> {
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
            let blob = BlobHeader {
                metadata_id: h.metadata_id & 0x7FFF_FFFF,
                ..h
            };
            f(&blob, payload)?;
        } else {
            let _event_size = c.i32()?;
            h.metadata_id = c.u32()? & 0x7FFF_FFFF;
            h.sequence = c.u32()?;
            h.thread = c.u64()?;
            h.capture_thread = c.u64()?;
            h.processor = c.u32()?;
            h.stack = c.u32()?;
            h.timestamp = c.u64()?;
            c.take(16 + 16)?; // ActivityID, RelatedActivityID
            h.payload_size = c.u32()?;
            let payload = c.take(h.payload_size as usize)?;
            f(&h, payload)?;
            // Blobs are aligned to 4 bytes; blocks start aligned, so the block offset is enough.
            let pad = (4 - c.pos % 4) % 4;
            c.take(pad.min(c.remaining()))?;
        }
    }
    Ok(())
}

/// One event type of the trace.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EventMeta {
    pub id: u32,
    pub provider: String,
    pub event_id: i32,
    pub event_name: String,
    pub version: i32,
    /// `None` when the field layout uses something this parser does not understand. Native
    /// runtime events usually carry no field list; their payloads are decoded by event id.
    pub fields: Option<Vec<Field>>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Field {
    pub name: String,
    pub ty: Ty,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Ty {
    /// A primitive of a fixed size, by its EventPipe type code.
    Fixed(u8, usize),
    /// UTF-16, null terminated.
    Str,
    Obj(Vec<Field>),
    /// `u16` count followed by the elements.
    Arr(Box<Ty>),
}

pub(crate) const MAX_DEPTH: usize = 16;
const MAX_FIELDS: usize = 4096;

/// Parses a metadata record (the payload of a MetadataBlock blob). `Ok(None)` for an empty record.
pub(crate) fn parse_metadata(payload: &[u8]) -> Result<Option<EventMeta>> {
    let mut c = Cursor::new(payload);
    if c.is_empty() {
        return Ok(None);
    }
    let id = c.u32()?;
    let provider = c.utf16z()?;
    let event_id = c.i32()?;
    let event_name = c.utf16z()?;
    let _keywords = c.u64()?;
    let version = c.i32()?;
    let _level = c.i32()?;
    let fields = parse_metadata_fields(&mut c).ok();
    Ok(Some(EventMeta {
        id,
        provider,
        event_id,
        event_name,
        version,
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
pub(crate) enum Value {
    Num(f64),
    Str(String),
    Obj(Vec<(String, Value)>),
    Arr(Vec<Value>),
    Other,
}

pub(crate) fn decode_fields(
    c: &mut Cursor,
    fields: &[Field],
    depth: usize,
) -> Result<Vec<(String, Value)>> {
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
pub(crate) struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub(crate) fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8]> {
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

    pub(crate) fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub(crate) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub(crate) fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    pub(crate) fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    /// An unsigned integer of `size` bytes (a pointer of the traced process: 4 or 8).
    pub(crate) fn uint(&mut self, size: usize) -> Result<u64> {
        match size {
            4 => self.u32().map(u64::from),
            _ => self.u64(),
        }
    }

    pub(crate) fn var_u64(&mut self) -> Result<u64> {
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

    pub(crate) fn var_u32(&mut self) -> Result<u32> {
        u32::try_from(self.var_u64()?).map_err(|_| anyhow!("varint out of range in nettrace data"))
    }

    /// UTF-16LE up to (and consuming) a null code unit.
    pub(crate) fn utf16z(&mut self) -> Result<String> {
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

/// Builders of synthetic NetTrace streams for unit tests.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub(crate) fn utf16z(out: &mut Vec<u8>, s: &str) {
        for u in s.encode_utf16().chain(std::iter::once(0)) {
            out.extend_from_slice(&u.to_le_bytes());
        }
    }

    pub(crate) fn i32le(out: &mut Vec<u8>, v: i32) {
        out.extend_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn varint(out: &mut Vec<u8>, mut v: u64) {
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

    pub(crate) fn object_type(out: &mut Vec<u8>, name: &str) {
        out.extend_from_slice(&[TAG_BEGIN_PRIVATE_OBJECT, TAG_BEGIN_PRIVATE_OBJECT, TAG_NULL]);
        i32le(out, 2);
        i32le(out, 2);
        i32le(out, name.len() as i32);
        out.extend_from_slice(name.as_bytes());
        out.push(TAG_END_OBJECT);
    }

    pub(crate) fn block(out: &mut Vec<u8>, name: &str, data: &[u8]) {
        object_type(out, name);
        i32le(out, data.len() as i32);
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
        out.extend_from_slice(data);
        out.push(TAG_END_OBJECT);
    }

    /// One compressed event blob for [`compressed_events`].
    pub(crate) struct Blob {
        pub metadata_id: u32,
        pub thread: u64,
        pub stack: u32,
        pub payload: Vec<u8>,
    }

    /// Block header (20 bytes, compressed) followed by compressed blobs `(metadata_id, payload)`.
    pub(crate) fn compressed_blobs(blobs: &[(u32, Vec<u8>)]) -> Vec<u8> {
        compressed_events(
            &blobs
                .iter()
                .map(|(id, p)| Blob {
                    metadata_id: *id,
                    thread: 0,
                    stack: 0,
                    payload: p.clone(),
                })
                .collect::<Vec<_>>(),
        )
    }

    /// Like [`compressed_blobs`], with the thread and stack id of each event.
    pub(crate) fn compressed_events(blobs: &[Blob]) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&20u16.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&[0u8; 16]);
        for b in blobs {
            d.push(1 | 4 | 8 | 128);
            varint(&mut d, b.metadata_id as u64);
            varint(&mut d, b.thread);
            varint(&mut d, b.stack as u64);
            varint(&mut d, 10); // timestamp delta
            varint(&mut d, b.payload.len() as u64);
            d.extend_from_slice(&b.payload);
        }
        d
    }

    /// A StackBlock with 8-byte instruction pointers.
    pub(crate) fn stack_block(first_id: u32, stacks: &[&[u64]]) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&first_id.to_le_bytes());
        d.extend_from_slice(&(stacks.len() as u32).to_le_bytes());
        for s in stacks {
            d.extend_from_slice(&((s.len() * 8) as u32).to_le_bytes());
            for ip in *s {
                d.extend_from_slice(&ip.to_le_bytes());
            }
        }
        d
    }

    pub(crate) fn metadata_header(id: u32, provider: &str, name: &str) -> Vec<u8> {
        metadata_with_event_id(id, provider, 0, name)
    }

    /// A metadata record without fields for event `event_id` of `provider`.
    pub(crate) fn metadata_with_event_id(
        id: u32,
        provider: &str,
        event_id: i32,
        name: &str,
    ) -> Vec<u8> {
        let mut m = Vec::new();
        i32le(&mut m, id as i32);
        utf16z(&mut m, provider);
        i32le(&mut m, event_id);
        utf16z(&mut m, name);
        m.extend_from_slice(&0u64.to_le_bytes());
        i32le(&mut m, 0);
        i32le(&mut m, 4);
        m
    }

    /// A full stream (pointer size 8) with the given blocks.
    pub(crate) fn stream(blocks: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut s = b"Nettrace".to_vec();
        i32le(&mut s, 20);
        s.extend_from_slice(b"!FastSerialization.1");
        object_type(&mut s, "Trace");
        s.extend_from_slice(&[0u8; 32]);
        i32le(&mut s, 8);
        s.extend_from_slice(&[0u8; 12]);
        s.push(TAG_END_OBJECT);
        for (name, data) in blocks {
            block(&mut s, name, data);
        }
        s.push(TAG_NULL);
        s
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

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
    fn unknown_field_types_drop_only_the_fields() {
        let mut m = metadata_header(5, "X", "Y");
        i32le(&mut m, 1);
        i32le(&mut m, 99);
        utf16z(&mut m, "Weird");
        let meta = parse_metadata(&m).unwrap().unwrap();
        assert_eq!(meta.fields, None);
    }

    #[derive(Default)]
    struct Recorder {
        pointer_size: usize,
        events: Vec<(i32, EventHeader, Vec<u8>)>,
        stacks: Vec<(u32, Vec<u64>)>,
    }

    impl Sink for Recorder {
        fn trace(&mut self, pointer_size: usize) {
            self.pointer_size = pointer_size;
        }
        fn event(&mut self, meta: &EventMeta, header: &EventHeader, payload: &[u8]) -> Result<()> {
            self.events.push((meta.event_id, *header, payload.to_vec()));
            Ok(())
        }
        fn stack(&mut self, id: u32, ips: &[u64]) {
            self.stacks.push((id, ips.to_vec()));
        }
    }

    #[test]
    fn delivers_event_headers_and_stacks() {
        let meta = compressed_blobs(&[(0, metadata_with_event_id(3, "P", 42, "E"))]);
        let events = compressed_events(&[
            Blob {
                metadata_id: 3,
                thread: 77,
                stack: 5,
                payload: vec![1, 2],
            },
            Blob {
                metadata_id: 9, // unknown: skipped
                thread: 78,
                stack: 0,
                payload: vec![],
            },
        ]);
        let bytes = stream(&[
            ("MetadataBlock", meta),
            ("StackBlock", stack_block(5, &[&[0x10, 0x20], &[]])),
            ("EventBlock", events),
        ]);
        let mut r = Recorder::default();
        parse_nettrace(bytes.as_slice(), &mut r).unwrap();
        assert_eq!(r.pointer_size, 8);
        assert_eq!(r.stacks, [(5, vec![0x10, 0x20]), (6, vec![])]);
        assert_eq!(r.events.len(), 1);
        let (id, h, p) = &r.events[0];
        assert_eq!(
            (*id, h.thread, h.stack, p.as_slice()),
            (42, 77, 5, &[1u8, 2][..])
        );
    }

    #[test]
    fn ipc_messages() {
        let payload = collect_tracing2_payload(
            10,
            true,
            &[Provider {
                name: "P",
                keywords: 0x10,
                level: 5,
                args: None,
            }],
        );
        // buffer, format, rundown, count, keywords, level, "P\0" (2 units), null args.
        assert_eq!(payload.len(), 4 + 4 + 1 + 4 + 8 + 4 + 4 + 4 + 4);
        assert_eq!(payload[8], 1);
        let m = ipc_message(EVENTPIPE, COLLECT_TRACING2, &payload).unwrap();
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
