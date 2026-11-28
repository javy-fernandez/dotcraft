//! The small part of BSON the Avalonia designer protocol uses, written the way Avalonia's
//! embedded serializer (Metsys.Bson) writes it: documents of doubles, strings, embedded
//! documents, arrays, binary data, booleans, nulls and 32/64-bit integers. Byte arrays use the
//! old binary subtype 2 (an inner length after the subtype), which is what Metsys reads back.

use std::fmt;

/// A BSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Double(f64),
    String(String),
    Document(Document),
    Array(Vec<Value>),
    Binary(Vec<u8>),
    Bool(bool),
    Null,
    Int32(i32),
    Int64(i64),
}

/// A document: its elements in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document(pub Vec<(String, Value)>);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BSON: {}", self.0)
    }
}

impl std::error::Error for Error {}

impl Document {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an element (builder style).
    pub fn with(mut self, name: &str, value: Value) -> Self {
        self.0.push((name.to_string(), value));
        self
    }

    pub fn get(&self, name: &str) -> Option<&Value> {
        self.0.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// A number of any BSON numeric type.
    pub fn f64(&self, name: &str) -> Option<f64> {
        match self.get(name)? {
            Value::Double(v) => Some(*v),
            Value::Int32(v) => Some(f64::from(*v)),
            Value::Int64(v) => Some(*v as f64),
            _ => None,
        }
    }

    /// An integer stored as Int32, Int64 or a whole Double.
    pub fn i64(&self, name: &str) -> Option<i64> {
        match self.get(name)? {
            Value::Int32(v) => Some(i64::from(*v)),
            Value::Int64(v) => Some(*v),
            Value::Double(v) if v.fract() == 0.0 => Some(*v as i64),
            _ => None,
        }
    }

    pub fn str(&self, name: &str) -> Option<&str> {
        match self.get(name)? {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn doc(&self, name: &str) -> Option<&Document> {
        match self.get(name)? {
            Value::Document(d) => Some(d),
            _ => None,
        }
    }

    pub fn array(&self, name: &str) -> Option<&[Value]> {
        match self.get(name)? {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Takes a binary element out of the document (frames carry megabytes of pixels).
    pub fn take_binary(&mut self, name: &str) -> Option<Vec<u8>> {
        self.0.iter_mut().find(|(n, _)| n == name).and_then(|(_, v)| match std::mem::replace(v, Value::Null) {
            Value::Binary(b) => Some(b),
            other => {
                *v = other;
                None
            }
        })
    }
}

/// Serializes a document.
pub fn encode(doc: &Document) -> Vec<u8> {
    let mut out = Vec::new();
    write_document(&mut out, doc.0.iter().map(|(n, v)| (n.as_str(), v)));
    out
}

fn write_document<'a>(out: &mut Vec<u8>, elements: impl Iterator<Item = (&'a str, &'a Value)>) {
    let start = out.len();
    out.extend_from_slice(&[0; 4]);
    for (name, value) in elements {
        write_element(out, name, value);
    }
    out.push(0);
    let len = (out.len() - start) as i32;
    out[start..start + 4].copy_from_slice(&len.to_le_bytes());
}

fn write_element(out: &mut Vec<u8>, name: &str, value: &Value) {
    let kind: u8 = match value {
        Value::Double(_) => 0x01,
        Value::String(_) => 0x02,
        Value::Document(_) => 0x03,
        Value::Array(_) => 0x04,
        Value::Binary(_) => 0x05,
        Value::Bool(_) => 0x08,
        Value::Null => 0x0A,
        Value::Int32(_) => 0x10,
        Value::Int64(_) => 0x12,
    };
    out.push(kind);
    out.extend_from_slice(name.as_bytes());
    out.push(0);
    match value {
        Value::Double(v) => out.extend_from_slice(&v.to_le_bytes()),
        Value::String(s) => {
            out.extend_from_slice(&(s.len() as i32 + 1).to_le_bytes());
            out.extend_from_slice(s.as_bytes());
            out.push(0);
        }
        Value::Document(d) => write_document(out, d.0.iter().map(|(n, v)| (n.as_str(), v))),
        Value::Array(items) => {
            let names: Vec<String> = (0..items.len()).map(|i| i.to_string()).collect();
            write_document(out, names.iter().map(String::as_str).zip(items.iter()));
        }
        Value::Binary(bytes) => {
            out.extend_from_slice(&(bytes.len() as i32 + 4).to_le_bytes());
            out.push(0x02);
            out.extend_from_slice(&(bytes.len() as i32).to_le_bytes());
            out.extend_from_slice(bytes);
        }
        Value::Bool(b) => out.push(u8::from(*b)),
        Value::Null => {}
        Value::Int32(v) => out.extend_from_slice(&v.to_le_bytes()),
        Value::Int64(v) => out.extend_from_slice(&v.to_le_bytes()),
    }
}

/// Parses a document that fills `bytes`.
pub fn decode(bytes: &[u8]) -> Result<Document, Error> {
    let mut r = Reader { bytes, pos: 0 };
    let doc = r.document()?;
    if r.pos != bytes.len() {
        return Err(Error(format!("{} bytes after the document", bytes.len() - r.pos)));
    }
    Ok(doc)
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.bytes.len()).ok_or_else(|| Error("unexpected end of data".into()))?;
        let s = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn i32(&mut self) -> Result<i32, Error> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn len(&mut self) -> Result<usize, Error> {
        let n = self.i32()?;
        usize::try_from(n).map_err(|_| Error(format!("negative length {n}")))
    }

    fn cstring(&mut self) -> Result<String, Error> {
        let rest = &self.bytes[self.pos..];
        let end = rest.iter().position(|&b| b == 0).ok_or_else(|| Error("unterminated name".into()))?;
        let s = String::from_utf8_lossy(&rest[..end]).into_owned();
        self.pos += end + 1;
        Ok(s)
    }

    fn document(&mut self) -> Result<Document, Error> {
        let start = self.pos;
        let len = self.len()?;
        let end = start.checked_add(len).filter(|&e| e <= self.bytes.len() && len >= 5).ok_or_else(|| Error(format!("bad document length {len}")))?;
        let mut doc = Document::new();
        while self.pos < end - 1 {
            let kind = self.take(1)?[0];
            let name = self.cstring()?;
            let value = self.value(kind)?;
            doc.0.push((name, value));
        }
        if self.pos != end - 1 || self.take(1)?[0] != 0 {
            return Err(Error("document does not end where its length says".into()));
        }
        Ok(doc)
    }

    fn value(&mut self, kind: u8) -> Result<Value, Error> {
        Ok(match kind {
            0x01 => Value::Double(f64::from_le_bytes(self.take(8)?.try_into().unwrap())),
            0x02 => {
                let len = self.len()?;
                if len == 0 {
                    return Err(Error("empty string length".into()));
                }
                let s = self.take(len)?;
                Value::String(String::from_utf8_lossy(&s[..len - 1]).into_owned())
            }
            0x03 => Value::Document(self.document()?),
            0x04 => Value::Array(self.document()?.0.into_iter().map(|(_, v)| v).collect()),
            0x05 => {
                let len = self.len()?;
                let subtype = self.take(1)?[0];
                let data = self.take(len)?;
                if subtype == 0x02 && len >= 4 {
                    let inner = usize::try_from(i32::from_le_bytes(data[..4].try_into().unwrap())).unwrap_or(usize::MAX);
                    Value::Binary(data[4..].get(..inner).ok_or_else(|| Error("bad binary length".into()))?.to_vec())
                } else {
                    Value::Binary(data.to_vec())
                }
            }
            0x08 => Value::Bool(self.take(1)?[0] != 0),
            0x0A => Value::Null,
            0x10 => Value::Int32(self.i32()?),
            0x12 => Value::Int64(i64::from_le_bytes(self.take(8)?.try_into().unwrap())),
            other => return Err(Error(format!("unsupported element type 0x{other:02x}"))),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_like_the_bson_spec_example() {
        // {"hello": "world"} from bsonspec.org.
        let doc = Document::new().with("hello", Value::String("world".into()));
        assert_eq!(encode(&doc), b"\x16\x00\x00\x00\x02hello\x00\x06\x00\x00\x00world\x00\x00");
    }

    #[test]
    fn round_trips_every_supported_type() {
        let doc = Document::new()
            .with("D", Value::Double(1.5))
            .with("S", Value::String("Grüße".into()))
            .with("O", Value::Document(Document::new().with("I", Value::Int32(-3))))
            .with("A", Value::Array(vec![Value::Int32(1), Value::Int32(2)]))
            .with("B", Value::Binary(vec![1, 2, 3]))
            .with("T", Value::Bool(true))
            .with("N", Value::Null)
            .with("L", Value::Int64(1 << 40));
        let bytes = encode(&doc);
        assert_eq!(decode(&bytes).unwrap(), doc);
    }

    #[test]
    fn binary_uses_the_old_subtype_with_an_inner_length() {
        let bytes = encode(&Document::new().with("Data", Value::Binary(vec![0xAA, 0xBB])));
        // type, "Data\0", total length (2 + 4), subtype 2, inner length 2, the bytes.
        assert_eq!(&bytes[4..], b"\x05Data\x00\x06\x00\x00\x00\x02\x02\x00\x00\x00\xAA\xBB\x00");
        // A generic (subtype 0) binary from another writer reads as well.
        let generic = b"\x12\x00\x00\x00\x05Data\x00\x02\x00\x00\x00\x00\xAA\xBB\x00";
        assert_eq!(decode(generic).unwrap().get("Data"), Some(&Value::Binary(vec![0xAA, 0xBB])));
    }

    #[test]
    fn arrays_are_documents_with_index_names() {
        let bytes = encode(&Document::new().with("Formats", Value::Array(vec![Value::Int32(1)])));
        assert_eq!(&bytes[4..], b"\x04Formats\x00\x0C\x00\x00\x00\x100\x00\x01\x00\x00\x00\x00\x00");
    }

    #[test]
    fn rejects_truncated_and_inconsistent_data() {
        let bytes = encode(&Document::new().with("hello", Value::String("world".into())));
        assert!(decode(&bytes[..bytes.len() - 1]).is_err());
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(decode(&longer).is_err());
        assert!(decode(b"\x05\x00").is_err());
        assert!(decode(b"\x0A\x00\x00\x00\x7Fx\x00\x00\x00\x00").is_err());
    }

    #[test]
    fn numeric_accessors_accept_any_numeric_type() {
        let doc = Document::new().with("a", Value::Int32(2)).with("b", Value::Double(3.0)).with("c", Value::Int64(4)).with("d", Value::String("x".into()));
        assert_eq!((doc.i64("a"), doc.i64("b"), doc.i64("c"), doc.i64("d")), (Some(2), Some(3), Some(4), None));
        assert_eq!(doc.f64("c"), Some(4.0));
    }
}
