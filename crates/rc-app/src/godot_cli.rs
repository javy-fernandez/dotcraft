//! Godot's external editor arguments and the local handoff to a running DotCraft window.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct OpenRequest {
    pub project: Option<PathBuf>,
    pub file: PathBuf,
    /// Zero-based editor coordinates. Generic editor placeholders are one-based; C# custom
    /// editor placeholders already use zero-based coordinates.
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Invocation {
    Ordinary(Option<PathBuf>),
    Godot(Result<OpenRequest, String>),
}

static PENDING: Mutex<Vec<OpenRequest>> = Mutex::new(Vec::new());
static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

pub fn parse(args: &[OsString]) -> Invocation {
    if args.first().is_some_and(|arg| arg == "--godot") {
        return Invocation::Godot(parse_godot(&args[1..], true));
    }
    if args.first().is_some_and(|arg| arg == "--godot-csharp") {
        return Invocation::Godot(parse_godot(&args[1..], false));
    }
    let open = args
        .first()
        .map(PathBuf::from)
        .filter(|path| !path.to_string_lossy().starts_with("-psn_"));
    Invocation::Ordinary(open)
}

fn parse_godot(args: &[OsString], one_based: bool) -> Result<OpenRequest, String> {
    if args.len() != 4 {
        return Err("expected --godot[-csharp] <project> <file> <line> <column>".into());
    }
    let project = PathBuf::from(&args[0]);
    let file = PathBuf::from(&args[1]);
    if file.as_os_str().is_empty() {
        return Err("Godot file path is empty".into());
    }
    let line = parse_coordinate(&args[2], "line", one_based)?;
    let col = parse_coordinate(&args[3], "column", one_based)?;
    let project = (!project.as_os_str().is_empty()).then_some(project);
    let file = if file.is_absolute() {
        file
    } else if let Some(root) = &project {
        root.join(file)
    } else {
        file
    };
    Ok(OpenRequest {
        project,
        file,
        line,
        col,
    })
}

fn parse_coordinate(value: &OsString, name: &str, one_based: bool) -> Result<usize, String> {
    if value.is_empty() {
        return Ok(0);
    }
    let display = value.to_string_lossy();
    if display == "-1" {
        return Ok(0);
    }
    let position = display
        .parse::<usize>()
        .map_err(|_| format!("invalid Godot {name}: {display}"))?;
    Ok(if one_based { position.saturating_sub(1) } else { position })
}

pub fn set_context(ctx: &egui::Context) {
    let _ = CONTEXT.set(ctx.clone());
}

pub fn queue(request: OpenRequest) {
    PENDING
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .push(request);
    if let Some(ctx) = CONTEXT.get() {
        ctx.request_repaint();
    }
}

pub fn take() -> Vec<OpenRequest> {
    std::mem::take(&mut *PENDING.lock().unwrap_or_else(|error| error.into_inner()))
}

#[cfg(unix)]
fn socket_path() -> Option<PathBuf> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let uid = unsafe { libc::geteuid() };
    let dir = std::env::temp_dir().join(format!("dotcraft-{uid}"));
    if let Err(error) = std::fs::create_dir(&dir)
        && error.kind() != std::io::ErrorKind::AlreadyExists
    {
        return None;
    }
    let metadata = std::fs::symlink_metadata(&dir).ok()?;
    if !metadata.file_type().is_dir() || metadata.uid() != uid {
        return None;
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).ok()?;
    }
    Some(dir.join("godot-editor.sock"))
}

#[cfg(unix)]
pub fn forward(request: &OpenRequest) -> bool {
    let Some(path) = socket_path() else {
        return false;
    };
    forward_to(&path, request)
}

#[cfg(unix)]
fn forward_to(path: &std::path::Path, request: &OpenRequest) -> bool {
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    let Ok(mut stream) = UnixStream::connect(path) else {
        return false;
    };
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(2)));
    let Ok(payload) = serde_json::to_vec(request) else {
        return false;
    };
    stream.write_all(&payload).is_ok()
}

#[cfg(not(unix))]
pub fn forward(_request: &OpenRequest) -> bool {
    false
}

#[cfg(unix)]
pub fn install_listener() {
    use std::io::Read;
    use std::os::unix::fs::FileTypeExt;
    use std::os::unix::net::{UnixListener, UnixStream};

    let Some(path) = socket_path() else { return };
    if let Ok(metadata) = std::fs::symlink_metadata(&path)
        && metadata.file_type().is_socket()
        && UnixStream::connect(&path).is_err_and(|error| error.kind() == std::io::ErrorKind::ConnectionRefused)
    {
        let _ = std::fs::remove_file(&path);
    }
    let Ok(listener) = UnixListener::bind(path) else {
        return;
    };
    let _ = std::thread::Builder::new()
        .name("godot-editor-handoff".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                let mut bytes = Vec::new();
                if stream.take(64 * 1024).read_to_end(&mut bytes).is_ok()
                    && let Ok(request) = serde_json::from_slice::<OpenRequest>(&bytes)
                {
                    queue(request);
                }
            }
        });
}

#[cfg(not(unix))]
pub fn install_listener() {}

#[cfg(test)]
mod tests {
    use super::{Invocation, OpenRequest, parse};
    use std::ffi::OsString;
    use std::path::PathBuf;

    #[test]
    fn godot_placeholders_open_file_at_zero_based_position() {
        let args = ["--godot", "/work/my game", "Scripts/Hero.cs", "12", "7"].map(OsString::from);
        assert_eq!(
            parse(&args),
            Invocation::Godot(Ok(OpenRequest {
                project: Some(PathBuf::from("/work/my game")),
                file: PathBuf::from("/work/my game/Scripts/Hero.cs"),
                line: 11,
                col: 6,
            }))
        );
    }

    #[test]
    fn godot_missing_coordinates_default_to_first_character() {
        let args = ["--godot", "/work/game", "/work/game/Hero.cs", "", ""].map(OsString::from);
        let Invocation::Godot(Ok(request)) = parse(&args) else {
            panic!("expected Godot request")
        };
        assert_eq!((request.line, request.col), (0, 0));
    }

    #[test]
    fn csharp_placeholders_preserve_zero_based_position() {
        let args = ["--godot-csharp", "/work/game", "/work/game/Hero.cs", "11", "6"].map(OsString::from);
        let Invocation::Godot(Ok(request)) = parse(&args) else {
            panic!("expected C# Godot request")
        };
        assert_eq!((request.line, request.col), (11, 6));
    }

    #[test]
    fn csharp_unknown_location_opens_first_character() {
        let args = ["--godot-csharp", "/work/game", "/work/game/Hero.cs", "-1", "-1"].map(OsString::from);
        let Invocation::Godot(Ok(request)) = parse(&args) else {
            panic!("expected C# Godot request")
        };
        assert_eq!((request.line, request.col), (0, 0));
    }

    #[test]
    fn invalid_godot_arguments_do_not_become_a_file_path() {
        let args = ["--godot", "/work/game", "Hero.cs", "oops", "3"].map(OsString::from);
        assert!(matches!(parse(&args), Invocation::Godot(Err(_))));
    }

    #[test]
    fn ordinary_file_argument_is_unchanged() {
        assert_eq!(
            parse(&[OsString::from("/work/a.cs")]),
            Invocation::Ordinary(Some(PathBuf::from("/work/a.cs")))
        );
        assert_eq!(
            parse(&[OsString::from("-psn_0_1")]),
            Invocation::Ordinary(None)
        );
    }

    #[cfg(unix)]
    #[test]
    fn local_handoff_preserves_file_and_position() {
        use std::io::Read;
        use std::os::unix::net::UnixListener;

        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("editor.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let request = OpenRequest {
            project: Some(PathBuf::from("/work/my game")),
            file: PathBuf::from("/work/my game/Hero.cs"),
            line: 11,
            col: 6,
        };
        let received = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes).unwrap();
            serde_json::from_slice::<OpenRequest>(&bytes).unwrap()
        });
        assert!(super::forward_to(&socket, &request));
        assert_eq!(received.join().unwrap(), request);
    }
}
