//! File masks for Find in Files, e.g. `*.cs, !*.Designer.cs`: comma- or semicolon-separated
//! globs, `!` excludes, case-insensitive. A glob without `/` matches the file name, one with
//! `/` the root-relative path.

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use std::fmt;

/// A parsed mask. The default mask accepts every file.
#[derive(Clone, Debug, Default)]
pub struct FileMask {
    include_names: Option<GlobSet>,
    include_paths: Option<GlobSet>,
    exclude_names: Option<GlobSet>,
    exclude_paths: Option<GlobSet>,
}

/// A glob that does not parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaskError {
    pub glob: String,
    pub message: String,
}

impl fmt::Display for MaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid file mask '{}': {}", self.glob, self.message)
    }
}

impl std::error::Error for MaskError {}

#[derive(Default)]
struct Sets {
    names: Vec<String>,
    paths: Vec<String>,
}

impl Sets {
    fn build(&self) -> Result<(Option<GlobSet>, Option<GlobSet>), MaskError> {
        Ok((build_set(&self.names)?, build_set(&self.paths)?))
    }
}

fn build_set(globs: &[String]) -> Result<Option<GlobSet>, MaskError> {
    if globs.is_empty() {
        return Ok(None);
    }
    let mut b = GlobSetBuilder::new();
    for g in globs {
        let glob = GlobBuilder::new(g)
            .case_insensitive(true)
            .literal_separator(true)
            .build()
            .map_err(|e| MaskError { glob: g.clone(), message: e.kind().to_string() })?;
        b.add(glob);
    }
    b.build().map(Some).map_err(|e| MaskError { glob: globs.join(", "), message: e.to_string() })
}

/// Splits on `,` and `;` outside `{...}` and `[...]`.
fn split_items(mask: &str) -> Vec<&str> {
    let (mut out, mut depth, mut start) = (Vec::new(), 0i32, 0);
    for (i, c) in mask.char_indices() {
        match c {
            '{' | '[' => depth += 1,
            '}' | ']' => depth -= 1,
            ',' | ';' if depth <= 0 => {
                out.push(&mask[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&mask[start..]);
    out
}

impl FileMask {
    /// Parses a mask; an empty or blank mask accepts every file.
    pub fn parse(mask: &str) -> Result<FileMask, MaskError> {
        let (mut inc, mut exc) = (Sets::default(), Sets::default());
        for item in split_items(mask).into_iter().map(str::trim).filter(|s| !s.is_empty()) {
            let (sets, glob) = match item.strip_prefix('!') {
                Some(g) => (&mut exc, g.trim()),
                None => (&mut inc, item),
            };
            if glob.is_empty() {
                continue;
            }
            let glob = glob.replace('\\', "/");
            if glob.contains('/') {
                let glob = glob.trim_start_matches('/');
                // `src/*.cs` is anchored at the root; `**/x/*.cs` anywhere.
                sets.paths.push(glob.to_string());
            } else {
                sets.names.push(glob);
            }
        }
        let (include_names, include_paths) = inc.build()?;
        let (exclude_names, exclude_paths) = exc.build()?;
        Ok(FileMask { include_names, include_paths, exclude_names, exclude_paths })
    }

    /// True when the mask accepts everything.
    pub fn is_all(&self) -> bool {
        self.include_names.is_none() && self.include_paths.is_none() && self.exclude_names.is_none() && self.exclude_paths.is_none()
    }

    /// Whether a root-relative, `/`-separated path passes the mask.
    pub fn matches(&self, rel: &str) -> bool {
        let name = rel.rsplit('/').next().unwrap_or(rel);
        let hit = |names: &Option<GlobSet>, paths: &Option<GlobSet>| {
            names.as_ref().is_some_and(|s| s.is_match(name)) || paths.as_ref().is_some_and(|s| s.is_match(rel))
        };
        if hit(&self.exclude_names, &self.exclude_paths) {
            return false;
        }
        (self.include_names.is_none() && self.include_paths.is_none()) || hit(&self.include_names, &self.include_paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_and_excludes() {
        let m = FileMask::parse("*.cs, !*.Designer.cs").unwrap();
        assert!(m.matches("src/Form1.cs"));
        assert!(m.matches("src/FORM1.CS"));
        assert!(!m.matches("src/Form1.Designer.cs"));
        assert!(!m.matches("src/app.json"));
    }

    #[test]
    fn empty_and_exclude_only() {
        assert!(FileMask::parse("").unwrap().is_all());
        assert!(FileMask::parse(" , ").unwrap().matches("x.txt"));
        let m = FileMask::parse("!*.min.js").unwrap();
        assert!(m.matches("a.js"));
        assert!(!m.matches("lib/a.min.js"));
    }

    #[test]
    fn path_globs_and_separators() {
        let m = FileMask::parse("src/*.cs; **/Tests/**").unwrap();
        assert!(m.matches("src/A.cs"));
        assert!(!m.matches("src/sub/A.cs"));
        assert!(m.matches("a/Tests/b/c.txt"));
        assert!(!m.matches("lib/A.cs"));
        let m = FileMask::parse("*.{cs,csproj}").unwrap();
        assert!(m.matches("App.csproj") && m.matches("x/A.cs") && !m.matches("A.vb"));
    }

    #[test]
    fn bad_glob() {
        let e = FileMask::parse("*.cs, a[").unwrap_err();
        assert_eq!(e.glob, "a[");
    }
}
