//! Messages of Avalonia's remote designer protocol (`Avalonia.Remote.Protocol`, the
//! `tcp-bson` transport): each message is a little-endian `i32` body length, the message
//! type's GUID in .NET byte order (`Guid.ToByteArray`), then the BSON body whose element
//! names are the C# property names.

use super::bson::{self, Document, Value};
use std::io::{self, Read};

/// `Avalonia.Remote.Protocol.Viewport.PixelFormat`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Rgb565 = 0,
    Rgba8888 = 1,
    Bgra8888 = 2,
}

impl PixelFormat {
    fn from_i64(v: i64) -> Option<Self> {
        match v {
            0 => Some(Self::Rgb565),
            1 => Some(Self::Rgba8888),
            2 => Some(Self::Bgra8888),
            _ => None,
        }
    }
}

/// A rendered frame (`FrameMessage`): `height` rows of `stride` bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub sequence_id: i64,
    pub format: PixelFormat,
    pub data: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub dpi_x: f64,
    pub dpi_y: f64,
}

impl Frame {
    /// The pixels as tightly packed premultiplied RGBA (what the previewer renders with Skia).
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.width * self.height * 4);
        for y in 0..self.height {
            let Some(row) = self.data.get(y * self.stride..) else { break };
            match self.format {
                PixelFormat::Rgba8888 => out.extend_from_slice(&row[..(self.width * 4).min(row.len())]),
                PixelFormat::Bgra8888 => {
                    for px in row.chunks_exact(4).take(self.width) {
                        out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
                    }
                }
                PixelFormat::Rgb565 => {
                    for px in row.chunks_exact(2).take(self.width) {
                        let v = u16::from_le_bytes([px[0], px[1]]);
                        let (r, g, b) = ((v >> 11) & 0x1F, (v >> 5) & 0x3F, v & 0x1F);
                        out.extend_from_slice(&[(r * 255 / 31) as u8, (g * 255 / 63) as u8, (b * 255 / 31) as u8, 255]);
                    }
                }
            }
        }
        out.resize(self.width * self.height * 4, 0);
        out
    }
}

/// `ExceptionDetails` of a failed XAML update. Line and position are 1-based.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExceptionDetails {
    pub exception_type: String,
    pub message: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// Host → IDE, first message after connecting.
    StartDesignerSession { session_id: String },
    /// IDE → host: the pixel formats the IDE can show.
    ClientSupportedPixelFormats { formats: Vec<PixelFormat> },
    /// IDE → host: the display scale (96 DPI is 100%).
    ClientRenderInfo { dpi_x: f64, dpi_y: f64 },
    /// IDE → host: the viewport size (the previewer only takes the DPI from it).
    ClientViewportAllocated { width: f64, height: f64, dpi_x: f64, dpi_y: f64 },
    /// IDE → host: the XAML to show, the assembly it belongs to and its path in the project
    /// (`/Views/MainWindow.axaml`).
    UpdateXaml { xaml: String, assembly_path: String, xaml_file_project_path: String },
    /// Host → IDE: the result of the last update; `error` is set when it failed.
    UpdateXamlResult { error: Option<String>, handle: Option<String>, exception: Option<ExceptionDetails> },
    /// Host → IDE: the size the previewed window wants, in physical pixels.
    RequestViewportResize { width: f64, height: f64 },
    /// Host → IDE: a rendered frame.
    Frame(Frame),
    /// IDE → host: the frame was shown; the next one is only sent after this.
    FrameReceived { sequence_id: i64 },
    /// A message this client does not use.
    Other { guid: [u8; 16] },
}

const START_DESIGNER_SESSION: &str = "854887CF-2694-4EB6-B499-7461B6FB96C7";
const UPDATE_XAML: &str = "9AEC9A2E-6315-4066-B4BA-E9A9EFD0F8CC";
const UPDATE_XAML_RESULT: &str = "B7A70093-0C5D-47FD-9261-22086D43A2E2";
const CLIENT_VIEWPORT_ALLOCATED: &str = "BD7A8DE6-3DB8-4A13-8583-D6D4AB189A31";
const REQUEST_VIEWPORT_RESIZE: &str = "9B47B3D8-61DF-4C38-ACD4-8C1BB72554AC";
const CLIENT_SUPPORTED_PIXEL_FORMATS: &str = "63481025-7016-43FE-BADC-F2FD0F88609E";
const CLIENT_RENDER_INFO: &str = "7A3C25D3-3652-438D-8EF1-86E942CC96C0";
const FRAME_RECEIVED: &str = "68014F8A-289D-4851-8D34-5367EDA7F827";
const FRAME: &str = "F58313EE-FE69-4536-819D-F52EDF201A0E";

/// A GUID in .NET's `ToByteArray` order: the first three groups little-endian, the rest as written.
pub fn guid_bytes(guid: &str) -> [u8; 16] {
    let hex: Vec<u8> = guid.bytes().filter(u8::is_ascii_hexdigit).collect();
    assert_eq!(hex.len(), 32, "malformed GUID {guid}");
    let byte = |i: usize| {
        let digit = |c: u8| (c as char).to_digit(16).unwrap() as u8;
        digit(hex[2 * i]) << 4 | digit(hex[2 * i + 1])
    };
    let mut out = [0u8; 16];
    let order = [3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15];
    for (o, i) in out.iter_mut().zip(order) {
        *o = byte(i);
    }
    out
}

fn string(s: &str) -> Value {
    Value::String(s.to_string())
}

impl Message {
    fn guid(&self) -> [u8; 16] {
        match self {
            Message::StartDesignerSession { .. } => guid_bytes(START_DESIGNER_SESSION),
            Message::ClientSupportedPixelFormats { .. } => guid_bytes(CLIENT_SUPPORTED_PIXEL_FORMATS),
            Message::ClientRenderInfo { .. } => guid_bytes(CLIENT_RENDER_INFO),
            Message::ClientViewportAllocated { .. } => guid_bytes(CLIENT_VIEWPORT_ALLOCATED),
            Message::UpdateXaml { .. } => guid_bytes(UPDATE_XAML),
            Message::UpdateXamlResult { .. } => guid_bytes(UPDATE_XAML_RESULT),
            Message::RequestViewportResize { .. } => guid_bytes(REQUEST_VIEWPORT_RESIZE),
            Message::Frame(_) => guid_bytes(FRAME),
            Message::FrameReceived { .. } => guid_bytes(FRAME_RECEIVED),
            Message::Other { guid } => *guid,
        }
    }

    fn body(&self) -> Document {
        let d = Document::new();
        match self {
            Message::StartDesignerSession { session_id } => d.with("SessionId", string(session_id)),
            Message::ClientSupportedPixelFormats { formats } => d.with("Formats", Value::Array(formats.iter().map(|f| Value::Int32(*f as i32)).collect())),
            Message::ClientRenderInfo { dpi_x, dpi_y } => d.with("DpiX", Value::Double(*dpi_x)).with("DpiY", Value::Double(*dpi_y)),
            Message::ClientViewportAllocated { width, height, dpi_x, dpi_y } => {
                d.with("Width", Value::Double(*width)).with("Height", Value::Double(*height)).with("DpiX", Value::Double(*dpi_x)).with("DpiY", Value::Double(*dpi_y))
            }
            Message::UpdateXaml { xaml, assembly_path, xaml_file_project_path } => {
                d.with("Xaml", string(xaml)).with("AssemblyPath", string(assembly_path)).with("XamlFileProjectPath", string(xaml_file_project_path))
            }
            Message::UpdateXamlResult { error, handle, exception } => {
                let opt = |s: &Option<String>| s.as_deref().map_or(Value::Null, string);
                let exception = exception.as_ref().map_or(Value::Null, |e| {
                    let num = |n: Option<u32>| n.map_or(Value::Null, |n| Value::Int32(n as i32));
                    Value::Document(
                        Document::new()
                            .with("ExceptionType", string(&e.exception_type))
                            .with("Message", string(&e.message))
                            .with("LineNumber", num(e.line))
                            .with("LinePosition", num(e.column)),
                    )
                });
                d.with("Error", opt(error)).with("Handle", opt(handle)).with("Exception", exception)
            }
            Message::RequestViewportResize { width, height } => d.with("Width", Value::Double(*width)).with("Height", Value::Double(*height)),
            Message::Frame(f) => d
                .with("SequenceId", Value::Int64(f.sequence_id))
                .with("Format", Value::Int32(f.format as i32))
                .with("Data", Value::Binary(f.data.clone()))
                .with("Width", Value::Int32(f.width as i32))
                .with("Height", Value::Int32(f.height as i32))
                .with("Stride", Value::Int32(f.stride as i32))
                .with("DpiX", Value::Double(f.dpi_x))
                .with("DpiY", Value::Double(f.dpi_y)),
            Message::FrameReceived { sequence_id } => d.with("SequenceId", Value::Int64(*sequence_id)),
            Message::Other { .. } => d,
        }
    }

    /// The message as sent on the wire (length, GUID, BSON).
    pub fn encode(&self) -> Vec<u8> {
        let body = bson::encode(&self.body());
        let mut out = Vec::with_capacity(20 + body.len());
        out.extend_from_slice(&(body.len() as i32).to_le_bytes());
        out.extend_from_slice(&self.guid());
        out.extend_from_slice(&body);
        out
    }

    /// Parses a message body of type `guid`.
    pub fn decode(guid: [u8; 16], body: &[u8]) -> Result<Message, bson::Error> {
        let mut d = bson::decode(body)?;
        let missing = |name: &str| bson::Error(format!("missing {name}"));
        let num = |d: &Document, name: &str| d.f64(name).ok_or_else(|| missing(name));
        let size = |d: &Document, name: &str| d.i64(name).and_then(|v| usize::try_from(v).ok()).ok_or_else(|| missing(name));
        let text = |d: &Document, name: &str| d.str(name).map(str::to_string);
        Ok(if guid == guid_bytes(START_DESIGNER_SESSION) {
            Message::StartDesignerSession { session_id: text(&d, "SessionId").unwrap_or_default() }
        } else if guid == guid_bytes(UPDATE_XAML_RESULT) {
            let exception = d.doc("Exception").map(|e| ExceptionDetails {
                exception_type: text(e, "ExceptionType").unwrap_or_default(),
                message: text(e, "Message").unwrap_or_default(),
                line: e.i64("LineNumber").and_then(|v| u32::try_from(v).ok()),
                column: e.i64("LinePosition").and_then(|v| u32::try_from(v).ok()),
            });
            Message::UpdateXamlResult { error: text(&d, "Error"), handle: text(&d, "Handle"), exception }
        } else if guid == guid_bytes(REQUEST_VIEWPORT_RESIZE) {
            Message::RequestViewportResize { width: num(&d, "Width")?, height: num(&d, "Height")? }
        } else if guid == guid_bytes(FRAME) {
            let format = d.i64("Format").and_then(PixelFormat::from_i64).ok_or_else(|| missing("Format"))?;
            let (width, height, stride) = (size(&d, "Width")?, size(&d, "Height")?, size(&d, "Stride")?);
            let data = d.take_binary("Data").unwrap_or_default();
            let row = width * if format == PixelFormat::Rgb565 { 2 } else { 4 };
            if height > 0 && (stride < row || data.len() < stride * (height - 1) + row) {
                return Err(bson::Error(format!("frame of {width}×{height} (stride {stride}) has {} bytes", data.len())));
            }
            Message::Frame(Frame { sequence_id: d.i64("SequenceId").unwrap_or_default(), format, data, width, height, stride, dpi_x: d.f64("DpiX").unwrap_or(96.0), dpi_y: d.f64("DpiY").unwrap_or(96.0) })
        } else if guid == guid_bytes(FRAME_RECEIVED) {
            Message::FrameReceived { sequence_id: d.i64("SequenceId").ok_or_else(|| missing("SequenceId"))? }
        } else if guid == guid_bytes(UPDATE_XAML) {
            Message::UpdateXaml { xaml: text(&d, "Xaml").unwrap_or_default(), assembly_path: text(&d, "AssemblyPath").unwrap_or_default(), xaml_file_project_path: text(&d, "XamlFileProjectPath").unwrap_or_default() }
        } else if guid == guid_bytes(CLIENT_RENDER_INFO) {
            Message::ClientRenderInfo { dpi_x: num(&d, "DpiX")?, dpi_y: num(&d, "DpiY")? }
        } else if guid == guid_bytes(CLIENT_VIEWPORT_ALLOCATED) {
            Message::ClientViewportAllocated { width: num(&d, "Width")?, height: num(&d, "Height")?, dpi_x: num(&d, "DpiX")?, dpi_y: num(&d, "DpiY")? }
        } else if guid == guid_bytes(CLIENT_SUPPORTED_PIXEL_FORMATS) {
            let formats = d.array("Formats").unwrap_or_default().iter().filter_map(|v| match v {
                Value::Int32(i) => PixelFormat::from_i64(i64::from(*i)),
                Value::Int64(i) => PixelFormat::from_i64(*i),
                _ => None,
            });
            Message::ClientSupportedPixelFormats { formats: formats.collect() }
        } else {
            Message::Other { guid }
        })
    }

    /// Reads one message; `Ok(None)` when the stream ended cleanly before it.
    pub fn read(r: &mut impl Read) -> io::Result<Option<Message>> {
        let mut head = [0u8; 20];
        match r.read_exact(&mut head[..1]) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
        r.read_exact(&mut head[1..])?;
        let len = i32::from_le_bytes(head[..4].try_into().unwrap());
        // Frames of a 4K window at 200% are about 130 MB; anything larger is a broken stream.
        let len = usize::try_from(len).ok().filter(|&l| l <= 512 << 20).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("bad message length {len}")))?;
        let guid: [u8; 16] = head[4..].try_into().unwrap();
        let mut body = vec![0u8; len];
        r.read_exact(&mut body)?;
        Message::decode(guid, &body).map(Some).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guids_use_the_dotnet_byte_order() {
        // new Guid("854887CF-2694-4EB6-B499-7461B6FB96C7").ToByteArray()
        assert_eq!(guid_bytes(START_DESIGNER_SESSION), [0xCF, 0x87, 0x48, 0x85, 0x94, 0x26, 0xB6, 0x4E, 0xB4, 0x99, 0x74, 0x61, 0xB6, 0xFB, 0x96, 0xC7]);
        assert_eq!(guid_bytes("7A3c25d3-3652-438D-8EF1-86E942CC96C0"), guid_bytes(CLIENT_RENDER_INFO));
    }

    #[test]
    fn frames_messages_with_length_and_guid() {
        let msg = Message::FrameReceived { sequence_id: 7 };
        let bytes = msg.encode();
        let body_len = i32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        assert_eq!(body_len, bytes.len() - 20);
        assert_eq!(&bytes[4..20], &guid_bytes(FRAME_RECEIVED));
        assert_eq!(bson::decode(&bytes[20..]).unwrap().get("SequenceId"), Some(&Value::Int64(7)));
        let mut r = io::Cursor::new([bytes.clone(), bytes].concat());
        assert_eq!(Message::read(&mut r).unwrap(), Some(msg.clone()));
        assert_eq!(Message::read(&mut r).unwrap(), Some(msg));
        assert_eq!(Message::read(&mut r).unwrap(), None);
    }

    #[test]
    fn client_messages_round_trip() {
        for msg in [
            Message::ClientSupportedPixelFormats { formats: vec![PixelFormat::Rgba8888, PixelFormat::Bgra8888] },
            Message::ClientRenderInfo { dpi_x: 192.0, dpi_y: 192.0 },
            Message::ClientViewportAllocated { width: 800.0, height: 600.0, dpi_x: 96.0, dpi_y: 96.0 },
            Message::UpdateXaml { xaml: "<UserControl/>".into(), assembly_path: "/p/bin/App.dll".into(), xaml_file_project_path: "/Views/A.axaml".into() },
            Message::StartDesignerSession { session_id: "s".into() },
            Message::RequestViewportResize { width: 400.0, height: 300.0 },
        ] {
            let mut r = io::Cursor::new(msg.encode());
            assert_eq!(Message::read(&mut r).unwrap(), Some(msg));
        }
    }

    #[test]
    fn reads_update_results_with_and_without_errors() {
        // Metsys writes every property; nulls included.
        let ok = Message::UpdateXamlResult { error: None, handle: Some("0".into()), exception: None };
        assert_eq!(Message::read(&mut io::Cursor::new(ok.encode())).unwrap(), Some(ok));
        let failed = Message::UpdateXamlResult {
            error: Some("Avalonia.Markup.Xaml.XamlLoadException: Unable to resolve type Buton".into()),
            handle: None,
            exception: Some(ExceptionDetails { exception_type: "XamlLoadException".into(), message: "Unable to resolve type Buton".into(), line: Some(5), column: Some(6) }),
        };
        assert_eq!(Message::read(&mut io::Cursor::new(failed.encode())).unwrap(), Some(failed));
    }

    #[test]
    fn decodes_frames_and_converts_their_pixels() {
        // 2×1 BGRA with a padded stride of 12 bytes.
        let frame = Frame { sequence_id: 3, format: PixelFormat::Bgra8888, data: vec![1, 2, 3, 255, 4, 5, 6, 128, 0, 0, 0, 0], width: 2, height: 1, stride: 12, dpi_x: 192.0, dpi_y: 192.0 };
        let msg = Message::Frame(frame.clone());
        let Some(Message::Frame(read)) = Message::read(&mut io::Cursor::new(msg.encode())).unwrap() else { panic!("not a frame") };
        assert_eq!(read, frame);
        assert_eq!(read.to_rgba(), vec![3, 2, 1, 255, 6, 5, 4, 128]);
        let rgba = Frame { format: PixelFormat::Rgba8888, ..frame.clone() };
        assert_eq!(rgba.to_rgba(), vec![1, 2, 3, 255, 4, 5, 6, 128]);
        // A frame whose data is shorter than its size says is rejected.
        let short = Message::Frame(Frame { data: vec![0; 4], ..frame });
        assert!(Message::read(&mut io::Cursor::new(short.encode())).is_err());
    }

    #[test]
    fn unknown_messages_are_kept_as_other() {
        let guid = guid_bytes("53778004-78fa-4381-8ec3-176a6f2328b6");
        let mut bytes = Vec::new();
        let body = bson::encode(&Document::new().with("Uri", Value::String("http://x".into())));
        bytes.extend_from_slice(&(body.len() as i32).to_le_bytes());
        bytes.extend_from_slice(&guid);
        bytes.extend_from_slice(&body);
        assert_eq!(Message::read(&mut io::Cursor::new(bytes)).unwrap(), Some(Message::Other { guid }));
    }
}
