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

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;

#[cfg(unix)]
pub use crate::eventpipe::diagnostic_sockets;
use crate::eventpipe::{
    Cursor, EventHeader, EventMeta, Field, MAX_DEPTH, Provider, Sink, StopRead, StreamEnded, Value,
    collect_tracing2_payload, connect_waiting, decode_fields, parse_nettrace, send_stop,
    set_read_timeout, start_session,
};

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
    let session = start_session(&mut conn, &counters_payload(interval_secs))?;
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

/// `CollectTracing2`: 10 MB buffer, no rundown, `System.Runtime` with every keyword at
/// Informational level (as `dotnet-counters` does).
fn counters_payload(interval_secs: u32) -> Vec<u8> {
    let args = format!("EventCounterIntervalSec={interval_secs}");
    collect_tracing2_payload(
        10,
        false,
        &[Provider {
            name: "System.Runtime",
            keywords: 0xFFFF_FFFF,
            level: 4,
            args: Some(&args),
        }],
    )
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

impl Sink for Aggregator<'_> {
    fn event(&mut self, meta: &EventMeta, _: &EventHeader, payload: &[u8]) -> Result<()> {
        if meta.event_name != "EventCounters" {
            return Ok(());
        }
        let Some(fields) = &meta.fields else {
            return Ok(());
        };
        if let Some((name, v)) = counter_value(fields, payload)? {
            self.counter(name, v);
        }
        Ok(())
    }

    fn block_end(&mut self) {
        if self.pending.len() >= self.expected {
            self.flush();
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eventpipe::test_support::*;
    use crate::eventpipe::{Ty, parse_metadata};

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
    fn counters_session_payload() {
        let p = counters_payload(1);
        assert_eq!(&p[..4], &10u32.to_le_bytes());
        assert_eq!(p[8], 0); // no rundown
        let args: Vec<u8> = "EventCounterIntervalSec=1\0"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert!(p.ends_with(&args));
    }
}
