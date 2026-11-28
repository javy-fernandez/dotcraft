//! A language server on another machine: the editor works on local copies under a
//! [`Mirror`], the server on the real files. [`UriMap`] rewrites every `file://` URI on the
//! way out (mirror → remote) and in (remote → mirror), in values and in object keys
//! (`WorkspaceEdit.changes` is keyed by URI). A remote file outside the opened folder (a
//! framework source, a decompiled type) maps into the mirror too and is fetched on demand.

use crate::rpc::MessageMap;
use crate::sync::{path_to_uri, uri_to_path};
use percent_encoding::percent_decode_str;
use rc_core::mirror::Mirror;
use serde_json::{Map, Value};
use std::path::Path;

pub struct UriMap {
    pub mirror: Mirror,
}

impl UriMap {
    pub fn to_remote(&self, uri: &str) -> Option<String> {
        if !uri.starts_with("file://") {
            return None;
        }
        let local = uri_to_path(&uri.parse().ok()?)?;
        let remote = self.mirror.remote(&local)?;
        Some(path_to_uri(Path::new(&remote)).as_str().to_string())
    }

    pub fn to_local(&self, uri: &str) -> Option<String> {
        // Remote paths are POSIX whatever this machine is: decode by hand.
        let rest = uri.strip_prefix("file://")?;
        let remote = percent_decode_str(rest).decode_utf8().ok()?;
        let local = self.mirror.local(&remote)?;
        Some(path_to_uri(&local).as_str().to_string())
    }
}

fn rewrite(value: &mut Value, map: &dyn Fn(&str) -> Option<String>) {
    match value {
        Value::String(s) => {
            if let Some(mapped) = map(s) {
                *s = mapped;
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|v| rewrite(v, map)),
        Value::Object(object) => {
            if object.keys().any(|k| k.starts_with("file://")) {
                let old = std::mem::take(object);
                let mut renamed = Map::new();
                for (k, v) in old {
                    renamed.insert(map(&k).unwrap_or(k), v);
                }
                *object = renamed;
            }
            object.values_mut().for_each(|v| rewrite(v, map));
        }
        _ => {}
    }
}

impl MessageMap for UriMap {
    fn outgoing(&self, message: &mut Value) {
        rewrite(message, &|s| self.to_remote(s));
    }

    fn incoming(&self, message: &mut Value) {
        rewrite(message, &|s| self.to_local(s));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn uris_move_between_the_mirror_and_the_remote_machine() {
        let mirror = std::env::temp_dir().join("dotcraft mirror");
        let map = UriMap { mirror: Mirror::new(&mirror) };
        let local = path_to_uri(&mirror.join("home/dev/My App/Program.cs")).as_str().to_string();
        let mut message = json!({
            "textDocument": { "uri": local },
            "rootUri": path_to_uri(&mirror.join("home/dev/My App")).as_str(),
            "other": "file:///elsewhere/x.cs",
            "text": "not a uri"
        });
        map.outgoing(&mut message);
        assert_eq!(message["textDocument"]["uri"], "file:///home/dev/My%20App/Program.cs");
        assert_eq!(message["rootUri"], "file:///home/dev/My%20App");
        // Not in the mirror: left alone.
        assert_eq!(message["other"], "file:///elsewhere/x.cs");
        assert_eq!(message["text"], "not a uri");
        let mut answer = json!({
            "changes": { "file:///home/dev/My%20App/Program.cs": [{ "newText": "x" }] },
            "location": { "uri": "file:///usr/share/dotnet/shared/System.Runtime.cs" }
        });
        map.incoming(&mut answer);
        let changes = answer["changes"].as_object().unwrap();
        assert_eq!(changes.keys().next().unwrap(), &local);
        assert_eq!(answer["location"]["uri"], path_to_uri(&mirror.join("usr/share/dotnet/shared/System.Runtime.cs")).as_str());
    }
}
