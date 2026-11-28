//! The few protobuf wire-format pieces the resource service needs: writing strings, booleans
//! and nested messages, and reading fields back (unknown ones are skipped).

/// A field value as it is on the wire.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value<'a> {
    Varint(u64),
    Fixed64(u64),
    Len(&'a [u8]),
    Fixed32(u32),
}

impl<'a> Value<'a> {
    pub fn as_str(&self) -> String {
        match self {
            Value::Len(bytes) => String::from_utf8_lossy(bytes).into_owned(),
            _ => String::new(),
        }
    }

    pub fn as_bytes(&self) -> &'a [u8] {
        match self {
            Value::Len(bytes) => bytes,
            _ => &[],
        }
    }

    pub fn as_u64(&self) -> u64 {
        match self {
            Value::Varint(v) | Value::Fixed64(v) => *v,
            Value::Fixed32(v) => u64::from(*v),
            Value::Len(_) => 0,
        }
    }

    pub fn as_bool(&self) -> bool {
        self.as_u64() != 0
    }

    pub fn as_f64(&self) -> f64 {
        match self {
            Value::Fixed64(v) => f64::from_bits(*v),
            _ => 0.0,
        }
    }
}

/// Iterates over the fields of a message: `(field number, value)`. Stops at malformed input.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    fn varint(&mut self) -> Option<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *self.buf.get(self.pos)?;
            self.pos += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let bytes = self.buf.get(self.pos..end)?;
        self.pos = end;
        Some(bytes)
    }
}

impl<'a> Iterator for Reader<'a> {
    type Item = (u32, Value<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.buf.len() {
            return None;
        }
        let key = self.varint()?;
        let field = (key >> 3) as u32;
        let value = match key & 7 {
            0 => Value::Varint(self.varint()?),
            1 => Value::Fixed64(u64::from_le_bytes(self.take(8)?.try_into().ok()?)),
            2 => {
                let n = usize::try_from(self.varint()?).ok()?;
                Value::Len(self.take(n)?)
            }
            5 => Value::Fixed32(u32::from_le_bytes(self.take(4)?.try_into().ok()?)),
            _ => {
                // Groups (3, 4) are not used by proto3: give up on the message.
                self.pos = self.buf.len();
                return None;
            }
        };
        Some((field, value))
    }
}

/// Builds a message.
#[derive(Default)]
pub struct Writer(pub Vec<u8>);

impl Writer {
    fn varint(&mut self, mut v: u64) {
        while v >= 0x80 {
            self.0.push((v as u8) | 0x80);
            v >>= 7;
        }
        self.0.push(v as u8);
    }

    pub fn bytes(&mut self, field: u32, bytes: &[u8]) -> &mut Self {
        self.varint(u64::from(field) << 3 | 2);
        self.varint(bytes.len() as u64);
        self.0.extend_from_slice(bytes);
        self
    }

    pub fn string(&mut self, field: u32, s: &str) -> &mut Self {
        self.bytes(field, s.as_bytes())
    }

    pub fn bool(&mut self, field: u32, b: bool) -> &mut Self {
        self.varint(u64::from(field) << 3);
        self.varint(u64::from(b));
        self
    }
}

/// A `google.protobuf.Value` as display text (lists joined with commas, structs as `{…}`).
pub fn value_text(buf: &[u8]) -> String {
    for (field, value) in Reader::new(buf) {
        return match field {
            1 => String::new(),
            2 => {
                let n = value.as_f64();
                if n.fract() == 0.0 && n.abs() < 1e15 { format!("{}", n as i64) } else { n.to_string() }
            }
            3 => value.as_str(),
            4 => value.as_bool().to_string(),
            5 => "{…}".to_string(),
            6 => Reader::new(value.as_bytes()).filter(|(f, _)| *f == 1).map(|(_, v)| value_text(v.as_bytes())).collect::<Vec<_>>().join(", "),
            _ => continue,
        };
    }
    String::new()
}

/// Splits a gRPC response body into its messages (5-byte prefix: compressed flag and length),
/// keeping an incomplete tail for the next chunk.
#[derive(Default)]
pub struct Frames {
    buf: Vec<u8>,
}

impl Frames {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        loop {
            if self.buf.len() < 5 {
                break;
            }
            let len = u32::from_be_bytes([self.buf[1], self.buf[2], self.buf[3], self.buf[4]]) as usize;
            if self.buf.len() < 5 + len {
                break;
            }
            out.push(self.buf[5..5 + len].to_vec());
            self.buf.drain(..5 + len);
        }
        out
    }
}

/// A message framed for a gRPC request body (uncompressed).
pub fn frame(message: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(message.len() + 5);
    out.push(0);
    out.extend_from_slice(&(message.len() as u32).to_be_bytes());
    out.extend_from_slice(message);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_reads_fields_back() {
        let mut inner = Writer::default();
        inner.string(1, "apiservice").bool(2, true);
        let mut w = Writer::default();
        w.string(1, "hello").bytes(3, &inner.0).bool(6, false);
        let fields: Vec<(u32, Value)> = Reader::new(&w.0).collect();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].1.as_str(), "hello");
        let nested: Vec<(u32, Value)> = Reader::new(fields[1].1.as_bytes()).collect();
        assert_eq!(nested[0].1.as_str(), "apiservice");
        assert!(nested[1].1.as_bool());
        assert_eq!(fields[2], (6, Value::Varint(0)));
    }

    #[test]
    fn long_strings_use_multi_byte_lengths_and_truncated_input_stops() {
        let long = "x".repeat(300);
        let mut w = Writer::default();
        w.string(13, &long);
        assert_eq!(Reader::new(&w.0).next().unwrap().1.as_str(), long);
        assert_eq!(Reader::new(&w.0[..w.0.len() - 1]).count(), 0);
    }

    #[test]
    fn protobuf_values_become_text() {
        let mut number = Writer::default();
        number.0.push(2 << 3 | 1);
        number.0.extend_from_slice(&4242.0f64.to_bits().to_le_bytes());
        assert_eq!(value_text(&number.0), "4242");
        let mut s = Writer::default();
        s.string(3, "/w/Api.csproj");
        assert_eq!(value_text(&s.0), "/w/Api.csproj");
        let mut list = Writer::default();
        let mut inner = Writer::default();
        inner.bytes(1, &s.0).bytes(1, &number.0);
        list.bytes(6, &inner.0);
        assert_eq!(value_text(&list.0), "/w/Api.csproj, 4242");
    }

    #[test]
    fn grpc_frames_are_split_across_chunks() {
        let mut body = frame(b"abc");
        body.extend(frame(b""));
        body.extend(frame(b"defgh"));
        let mut frames = Frames::default();
        assert_eq!(frames.push(&body[..6]), Vec::<Vec<u8>>::new());
        assert_eq!(frames.push(&body[6..13]), vec![b"abc".to_vec(), Vec::new()]);
        assert_eq!(frames.push(&body[13..]), vec![b"defgh".to_vec()]);
    }
}
