//! A debug adapter on another machine. Breakpoints and the editor use local copies under a
//! [`Mirror`]; the adapter works with the real paths. [`PathMap`] rewrites every string
//! that is a path in the mirror on the way out, and the `path` of sources and modules on
//! the way in, so a stop in any remote file (the opened folder or not) opens its local copy.

use crate::client::MessageMap;
use rc_core::mirror::Mirror;
use serde_json::Value;
use std::path::Path;

pub struct PathMap {
    pub mirror: Mirror,
}

fn outgoing(value: &mut Value, mirror: &Mirror) {
    match value {
        Value::String(s) => {
            if mirror.contains(Path::new(s.as_str()))
                && let Some(remote) = mirror.remote(Path::new(s.as_str()))
            {
                *s = remote;
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|v| outgoing(v, mirror)),
        Value::Object(object) => object.values_mut().for_each(|v| outgoing(v, mirror)),
        _ => {}
    }
}

fn incoming(value: &mut Value, mirror: &Mirror) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(|v| incoming(v, mirror)),
        Value::Object(object) => {
            for (key, v) in object.iter_mut() {
                if key == "path"
                    && let Value::String(s) = v
                    && let Some(local) = mirror.local(s)
                {
                    *s = local.to_string_lossy().into_owned();
                } else {
                    incoming(v, mirror);
                }
            }
        }
        _ => {}
    }
}

impl MessageMap for PathMap {
    fn outgoing(&self, message: &mut Value) {
        outgoing(message, &self.mirror);
    }

    fn incoming(&self, message: &mut Value) {
        incoming(message, &self.mirror);
    }
}

/// Folder of netcoredbg `release` on a remote machine (`~` is the remote home).
pub fn remote_dir(release: &str) -> String {
    format!("~/.dotcraft/remote/netcoredbg/{release}")
}

/// The netcoredbg release and asset for a remote machine, from its `uname -sm` and, on
/// Linux, its glibc version (`getconf GNU_LIBC_VERSION`: `glibc 2.35`).
pub fn remote_release(uname: &str, glibc: Option<&str>) -> Option<(&'static str, &'static str)> {
    let mut words = uname.split_whitespace();
    let (os, arch) = match (words.next()?, words.next()?) {
        ("Linux", "x86_64" | "amd64") => ("linux", "x86_64"),
        ("Linux", "aarch64" | "arm64") => ("linux", "aarch64"),
        ("Darwin", "arm64") => ("macos", "aarch64"),
        _ => return None,
    };
    let asset = crate::netcoredbg::asset_for(os, arch)?;
    let release = match glibc.and_then(parse_glibc).filter(|_| os == "linux") {
        Some(version) => crate::netcoredbg::release_for_glibc(version),
        None => crate::netcoredbg::VERSION,
    };
    Some((release, asset))
}

fn parse_glibc(text: &str) -> Option<(u32, u32)> {
    let version = text.split_whitespace().find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()))?;
    let mut parts = version.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.trim_end_matches(|c: char| !c.is_ascii_digit()).parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn breakpoints_go_out_with_remote_paths_and_stops_come_back_local() {
        let root = std::env::temp_dir().join("dotcraft-mirror");
        let map = PathMap { mirror: Mirror::new(&root) };
        let local = root.join("home/dev/App/Program.cs");
        let mut set = json!({ "source": { "name": "Program.cs", "path": local.to_string_lossy() }, "lines": [3] });
        map.outgoing(&mut set);
        assert_eq!(set["source"]["path"], "/home/dev/App/Program.cs");
        let mut launch = json!({ "program": "/usr/bin/dotnet", "args": ["/home/dev/App/bin/App.dll"] });
        map.outgoing(&mut launch);
        assert_eq!(launch["program"], "/usr/bin/dotnet");
        let mut frames = json!({ "stackFrames": [{ "name": "Main", "source": { "path": "/home/dev/App/Program.cs" }, "line": 3 }], "text": "/not/a/path/key" });
        map.incoming(&mut frames);
        assert_eq!(frames["stackFrames"][0]["source"]["path"], local.to_string_lossy().as_ref());
        assert_eq!(frames["text"], "/not/a/path/key");
    }

    #[test]
    fn the_remote_release_fits_its_glibc() {
        assert_eq!(remote_release("Linux x86_64", Some("glibc 2.39")), Some((crate::netcoredbg::VERSION, "netcoredbg-linux-amd64.tar.gz")));
        assert_eq!(remote_release("Linux aarch64", Some("glibc 2.35")), Some(("3.1.3-1062", "netcoredbg-linux-arm64.tar.gz")));
        assert_eq!(remote_release("Linux x86_64", Some("glibc 2.28")), Some(("3.0.0-1018", "netcoredbg-linux-amd64.tar.gz")));
        assert_eq!(remote_release("Darwin arm64", None), Some((crate::netcoredbg::VERSION, "netcoredbg-osx-arm64.zip")));
        assert_eq!(remote_release("Darwin x86_64", None), None);
        assert_eq!(remote_dir("3.1.3-1062"), "~/.dotcraft/remote/netcoredbg/3.1.3-1062");
    }
}
