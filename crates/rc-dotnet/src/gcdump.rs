//! Memory snapshot of a running .NET process (a light `dotnet-gcdump`), without external tools.
//!
//! [`snapshot`] starts an EventPipe session with `Microsoft-Windows-DotNETRuntime` and the
//! `GC | GCHeapCollect | GCHeapDump | Type | GCHeapAndTypeNames` keywords at Verbose. Enabling
//! `GCHeapCollect` makes the runtime run a full blocking GC that walks the heap and logs every
//! live object (`GCBulkNode`: address, size, type) and the types it meets (`GCBulkType`). Once the
//! `GCEnd` of that collection arrives the session is stopped, the rest of the stream drained and
//! the objects are grouped by type. Only shallow sizes are computed (retained sizes would need
//! the reference graph and a dominator tree).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Result, bail};

use crate::eventpipe::{
    Cursor, EventHeader, EventMeta, Provider, Sink, collect_tracing2_payload, run_session,
};

/// Live objects of one type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeStats {
    /// Full type name (`System.String`, `MyApp.Widget`, `System.Collections.Generic.List<MyApp.Widget>`,
    /// `System.Byte[]`).
    pub name: String,
    pub count: u64,
    /// Sum of the shallow sizes of the objects, in bytes.
    pub size_bytes: u64,
}

/// The live objects of a process after a full GC, grouped by type.
#[derive(Debug, Clone, PartialEq)]
pub struct HeapSnapshot {
    /// Sorted by `size_bytes`, largest first.
    pub types: Vec<TypeStats>,
    pub total_bytes: u64,
    pub total_objects: u64,
    pub taken_at: SystemTime,
}

const RUNTIME_PROVIDER: &str = "Microsoft-Windows-DotNETRuntime";
const KEYWORD_GC: u64 = 0x1;
const KEYWORD_TYPE: u64 = 0x8_0000;
const KEYWORD_GC_HEAP_DUMP: u64 = 0x10_0000;
const KEYWORD_GC_HEAP_COLLECT: u64 = 0x80_0000;
const KEYWORD_GC_HEAP_AND_TYPE_NAMES: u64 = 0x100_0000;

const EVENT_GC_START: i32 = 1;
const EVENT_GC_END: i32 = 2;
const EVENT_GC_BULK_TYPE: i32 = 15;
const EVENT_GC_BULK_NODE: i32 = 18;

/// `GCStart.Type` of a background GC (which does not walk the heap).
const GC_TYPE_BACKGROUND: u32 = 1;
/// `GCBulkType.Flags`: the type is an array.
const TYPE_FLAG_ARRAY: u32 = 0x8;

/// Takes a heap snapshot of `pid`: forces a full GC in the process and groups its live objects by
/// type. Blocking; it pauses the process for the length of the GC.
///
/// Waits up to 10 s for the diagnostic port to appear. Errors when the port cannot be reached,
/// the runtime rejects the session, or no heap dump arrives within `timeout`.
pub fn snapshot(pid: u32, timeout: Duration) -> Result<HeapSnapshot> {
    let payload = collect_tracing2_payload(
        256,
        false,
        &[Provider {
            name: RUNTIME_PROVIDER,
            keywords: KEYWORD_GC
                | KEYWORD_TYPE
                | KEYWORD_GC_HEAP_DUMP
                | KEYWORD_GC_HEAP_COLLECT
                | KEYWORD_GC_HEAP_AND_TYPE_NAMES,
            level: 5,
            args: None,
        }],
    );
    let stop = AtomicBool::new(false);
    let end = AtomicBool::new(false);
    let mut heap = HeapCollector::new(&end);
    let until = Instant::now() + timeout;
    run_session(
        pid,
        &payload,
        &stop,
        &end,
        Some(until),
        Duration::from_secs(10),
        &mut heap,
    )?;
    if !heap.complete {
        if heap.objects.is_empty() {
            bail!("the process did not produce a heap dump within {timeout:?}");
        }
        bail!("the heap dump did not complete within {timeout:?}");
    }
    Ok(heap.into_snapshot())
}

/// Collects the GC heap dump events of a session.
struct HeapCollector<'a> {
    end: &'a AtomicBool,
    pointer_size: usize,
    /// `Count` of the full blocking GC that walks the heap.
    dump_gc: Option<u32>,
    /// The `GCEnd` of `dump_gc` arrived.
    complete: bool,
    /// Objects and bytes per type id (MethodTable address).
    objects: HashMap<u64, (u64, u64)>,
    types: HashMap<u64, TypeInfo>,
}

#[derive(Debug, Clone, PartialEq)]
struct TypeInfo {
    name: String,
    flags: u32,
    params: Vec<u64>,
}

impl<'a> HeapCollector<'a> {
    fn new(end: &'a AtomicBool) -> Self {
        Self {
            end,
            pointer_size: 8,
            dump_gc: None,
            complete: false,
            objects: HashMap::new(),
            types: HashMap::new(),
        }
    }

    fn into_snapshot(self) -> HeapSnapshot {
        let mut by_name: HashMap<String, TypeStats> = HashMap::new();
        for (ty, (count, size)) in &self.objects {
            let name = type_name(&self.types, *ty, 0);
            let e = by_name.entry(name.clone()).or_insert(TypeStats {
                name,
                count: 0,
                size_bytes: 0,
            });
            e.count += count;
            e.size_bytes += size;
        }
        let mut types: Vec<TypeStats> = by_name.into_values().collect();
        types.sort_by(|a, b| {
            b.size_bytes
                .cmp(&a.size_bytes)
                .then_with(|| a.name.cmp(&b.name))
        });
        HeapSnapshot {
            total_bytes: types.iter().map(|t| t.size_bytes).sum(),
            total_objects: types.iter().map(|t| t.count).sum(),
            types,
            taken_at: SystemTime::now(),
        }
    }

    /// `GCBulkNode`: `Index u32, Count u32, ClrInstanceID u16`, then `Count` x
    /// `{ Address ptr, Size u64, TypeID ptr, EdgeCount u64 }`.
    fn bulk_node(&mut self, payload: &[u8]) -> Result<()> {
        let mut c = Cursor::new(payload);
        let _index = c.u32()?;
        let count = c.u32()?;
        let _clr = c.u16()?;
        let ptr = self.pointer_size;
        for _ in 0..count {
            let _address = c.uint(ptr)?;
            let size = c.u64()?;
            let ty = c.uint(ptr)?;
            let _edges = c.u64()?;
            let e = self.objects.entry(ty).or_default();
            e.0 += 1;
            e.1 += size;
        }
        Ok(())
    }

    /// `GCBulkType`: `Count u32, ClrInstanceID u16`, then `Count` x `{ TypeID u64, ModuleID u64,
    /// TypeNameID u32, Flags u32, CorElementType u8, Name utf16z, TypeParameterCount u32,
    /// TypeParameters u64[] }`.
    fn bulk_type(&mut self, payload: &[u8]) -> Result<()> {
        let mut c = Cursor::new(payload);
        let count = c.u32()?;
        let _clr = c.u16()?;
        for _ in 0..count {
            let id = c.u64()?;
            let _module = c.u64()?;
            let _name_id = c.u32()?;
            let flags = c.u32()?;
            let _element = c.u8()?;
            let name = c.utf16z()?;
            let n = c.u32()?;
            let params = (0..n).map(|_| c.u64()).collect::<Result<Vec<_>>>()?;
            self.types.insert(
                id,
                TypeInfo {
                    name,
                    flags,
                    params,
                },
            );
        }
        Ok(())
    }
}

impl Sink for HeapCollector<'_> {
    fn trace(&mut self, pointer_size: usize) {
        self.pointer_size = pointer_size;
    }

    fn event(&mut self, meta: &EventMeta, _: &EventHeader, payload: &[u8]) -> Result<()> {
        if meta.provider != RUNTIME_PROVIDER {
            return Ok(());
        }
        match meta.event_id {
            // GCStart_V1+: Count, Depth, Reason, Type, ...
            EVENT_GC_START if self.dump_gc.is_none() => {
                let mut c = Cursor::new(payload);
                let (Ok(count), Ok(depth), Ok(_reason), Ok(ty)) =
                    (c.u32(), c.u32(), c.u32(), c.u32())
                else {
                    return Ok(());
                };
                if depth == 2 && ty != GC_TYPE_BACKGROUND {
                    self.dump_gc = Some(count);
                }
            }
            // GCEnd_V1: Count, Depth, ...
            EVENT_GC_END => {
                let count = Cursor::new(payload).u32().ok();
                if count.is_some() && count == self.dump_gc {
                    self.complete = true;
                    self.end.store(true, Ordering::Relaxed);
                }
            }
            EVENT_GC_BULK_NODE => self.bulk_node(payload)?,
            EVENT_GC_BULK_TYPE => self.bulk_type(payload)?,
            _ => {}
        }
        Ok(())
    }
}

/// The display name of a type: generic arguments resolved, arrays with `[]`.
fn type_name(types: &HashMap<u64, TypeInfo>, id: u64, depth: usize) -> String {
    let Some(t) = types.get(&id) else {
        return format!("[type 0x{id:x}]");
    };
    if depth > 8 {
        return t.name.clone();
    }
    let arg = |p: &u64| type_name(types, *p, depth + 1);
    if t.flags & TYPE_FLAG_ARRAY != 0 {
        if !t.name.is_empty() && t.name.contains('[') {
            return t.name.clone();
        }
        let element = t.params.first().map(arg);
        return match element {
            Some(e) => format!("{e}[]"),
            None => format!("{}[]", t.name),
        };
    }
    if t.params.is_empty() || t.name.contains('<') {
        return t.name.clone();
    }
    // `System.Collections.Generic.List`1` -> `System.Collections.Generic.List<Widget>`.
    let base = match t.name.rfind('`') {
        Some(i) => &t.name[..i],
        None => &t.name,
    };
    let args: Vec<String> = t.params.iter().map(arg).collect();
    format!("{base}<{}>", args.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eventpipe::parse_nettrace;
    use crate::eventpipe::test_support::*;

    fn bulk_type_entry(out: &mut Vec<u8>, id: u64, name: &str, flags: u32, params: &[u64]) {
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.push(0);
        utf16z(out, name);
        out.extend_from_slice(&(params.len() as u32).to_le_bytes());
        for p in params {
            out.extend_from_slice(&p.to_le_bytes());
        }
    }

    fn bulk_types(entries: &[(u64, &str, u32, &[u64])]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes());
        for (id, name, flags, params) in entries {
            bulk_type_entry(&mut p, *id, name, *flags, params);
        }
        p
    }

    fn bulk_nodes(nodes: &[(u64, u64)]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u32.to_le_bytes());
        p.extend_from_slice(&(nodes.len() as u32).to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes());
        for (i, (size, ty)) in nodes.iter().enumerate() {
            p.extend_from_slice(&(0x1000 + i as u64 * 64).to_le_bytes());
            p.extend_from_slice(&size.to_le_bytes());
            p.extend_from_slice(&ty.to_le_bytes());
            p.extend_from_slice(&0u64.to_le_bytes());
        }
        p
    }

    fn gc_start(count: u32, depth: u32, ty: u32) -> Vec<u8> {
        let mut p = Vec::new();
        for v in [count, depth, 1, ty] {
            p.extend_from_slice(&v.to_le_bytes());
        }
        p.extend_from_slice(&0u16.to_le_bytes());
        p.extend_from_slice(&0u64.to_le_bytes());
        p
    }

    fn gc_end(count: u32) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&count.to_le_bytes());
        p.extend_from_slice(&2u32.to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes());
        p
    }

    #[test]
    fn groups_heap_dump_by_type() {
        let meta = compressed_blobs(&[
            (
                0,
                metadata_with_event_id(1, RUNTIME_PROVIDER, EVENT_GC_START, ""),
            ),
            (
                0,
                metadata_with_event_id(2, RUNTIME_PROVIDER, EVENT_GC_END, ""),
            ),
            (
                0,
                metadata_with_event_id(3, RUNTIME_PROVIDER, EVENT_GC_BULK_TYPE, ""),
            ),
            (
                0,
                metadata_with_event_id(4, RUNTIME_PROVIDER, EVENT_GC_BULK_NODE, ""),
            ),
        ]);
        let events = compressed_blobs(&[
            (1, gc_start(6, 0, 0)), // a gen0 GC first: ignored
            (2, gc_end(6)),
            (1, gc_start(7, 2, 0)),
            (
                4,
                bulk_nodes(&[(24, 0xA), (24, 0xA), (100, 0xB), (40, 0xC), (32, 0xD)]),
            ),
            (
                3,
                bulk_types(&[
                    (0xA, "App.Widget", 0, &[]),
                    (0xB, "System.Byte", TYPE_FLAG_ARRAY, &[0xE]),
                    (0xC, "System.Collections.Generic.List`1", 0, &[0xA]),
                    (0xE, "System.Byte", 0, &[]),
                ]),
            ),
            (2, gc_end(7)),
        ]);
        let end = AtomicBool::new(false);
        let mut heap = HeapCollector::new(&end);
        parse_nettrace(
            stream(&[("MetadataBlock", meta), ("EventBlock", events)]).as_slice(),
            &mut heap,
        )
        .unwrap();
        assert!(heap.complete && end.load(Ordering::Relaxed));
        let snap = heap.into_snapshot();
        assert_eq!(snap.total_objects, 5);
        assert_eq!(snap.total_bytes, 220);
        let names: Vec<(&str, u64, u64)> = snap
            .types
            .iter()
            .map(|t| (t.name.as_str(), t.count, t.size_bytes))
            .collect();
        assert_eq!(
            names,
            [
                ("System.Byte[]", 1, 100),
                ("App.Widget", 2, 48),
                ("System.Collections.Generic.List<App.Widget>", 1, 40),
                ("[type 0xd]", 1, 32),
            ]
        );
    }

    #[test]
    fn truncated_bulk_events_error() {
        let end = AtomicBool::new(false);
        let mut heap = HeapCollector::new(&end);
        let nodes = bulk_nodes(&[(24, 0xA)]);
        assert!(heap.bulk_node(&nodes[..nodes.len() - 1]).is_err());
        let types = bulk_types(&[(0xA, "W", 0, &[1, 2])]);
        assert!(heap.bulk_type(&types[..types.len() - 3]).is_err());
    }
}
