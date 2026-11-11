//! Work item links in commit messages: `#123` (GitHub / GitLab issues, Azure Repos work
//! items), `AB#123` (Azure Boards), `GH-123` and `owner/repo#123`, plus custom
//! `regex → URL template` patterns. The issue tracker is derived from the repository's remote
//! URL (HTTPS, SSH and scp-like forms of github.com, GitHub Enterprise, GitLab, dev.azure.com,
//! `*.visualstudio.com` and Azure DevOps Server). No I/O here.

use regex::Regex;
use std::ops::Range;

/// The service a remote is hosted on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrackerKind {
    GitHub,
    GitLab,
    Azure,
}

/// The issue tracker of a remote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tracker {
    pub kind: TrackerKind,
    /// The web page of the repository (GitHub, GitLab) or the project (Azure DevOps), e.g.
    /// `https://github.com/owner/repo` or `https://dev.azure.com/org/project`.
    pub web: String,
    /// The REST API root: `https://api.github.com/repos/owner/repo`,
    /// `https://host/api/v4/projects/group%2Frepo` or the Azure project URL.
    pub api: String,
    /// The host name, for token lookup (`github.com`, `dev.azure.com`…).
    pub host: String,
}

impl Tracker {
    /// The page of work item / issue `id`.
    pub fn item_url(&self, id: &str) -> String {
        match self.kind {
            TrackerKind::GitHub => format!("{}/issues/{id}", self.web),
            TrackerKind::GitLab => format!("{}/-/issues/{id}", self.web),
            TrackerKind::Azure => format!("{}/_workitems/edit/{id}", self.web),
        }
    }
}

/// Host, port and path of a remote URL (`scheme://user@host:port/path` or scp-like
/// `user@host:path`), without the trailing `.git` and slashes.
fn split_remote(url: &str) -> Option<(String, Option<u16>, Vec<String>)> {
    let url = url.trim();
    let (authority, path) = if let Some((_, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        (authority, path)
    } else {
        // scp-like: [user@]host:path (a Windows drive or a local path has no host).
        let (authority, path) = url.split_once(':')?;
        if authority.contains('/') || authority.len() < 2 {
            return None;
        }
        (authority, path)
    };
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => (h, p.parse().ok()),
        _ => (host_port, None),
    };
    if host.is_empty() {
        return None;
    }
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let segments: Vec<String> = path.split('/').filter(|s| !s.is_empty()).map(str::to_string).collect();
    Some((host.to_ascii_lowercase(), port, segments))
}

/// The issue tracker of a remote URL, if the host is a known service.
pub fn tracker_from_remote(url: &str) -> Option<Tracker> {
    let (host, port, seg) = split_remote(url)?;
    let is_ssh = url.starts_with("ssh://") || !url.contains("://");
    // Azure DevOps over SSH: ssh.dev.azure.com:v3/org/project/repo.
    if host == "ssh.dev.azure.com" && seg.len() >= 3 && seg[0] == "v3" {
        let web = format!("https://dev.azure.com/{}/{}", seg[1], seg[2]);
        return Some(Tracker { kind: TrackerKind::Azure, api: web.clone(), web, host: "dev.azure.com".into() });
    }
    // Old Azure DevOps SSH: org@vs-ssh.visualstudio.com:v3/org/project/repo.
    if host.ends_with("vs-ssh.visualstudio.com") && seg.len() >= 3 && seg[0] == "v3" {
        let web = format!("https://{}.visualstudio.com/{}", seg[1], seg[2]);
        return Some(Tracker { kind: TrackerKind::Azure, api: web.clone(), web, host: format!("{}.visualstudio.com", seg[1]) });
    }
    // Azure DevOps HTTPS (dev.azure.com, *.visualstudio.com, Azure DevOps Server): …/_git/repo.
    if let Some(git) = seg.iter().position(|s| s == "_git") {
        let mut before: Vec<String> = seg[..git].to_vec();
        // `https://dev.azure.com/org/_git/repo`: the project is named after the repository.
        let org_level = if host == "dev.azure.com" { 1 } else if host.ends_with(".visualstudio.com") { usize::from(before.first().is_some_and(|s| s.eq_ignore_ascii_case("DefaultCollection"))) } else { usize::MAX };
        if before.len() == org_level
            && let Some(repo) = seg.get(git + 1)
        {
            before.push(repo.clone());
        }
        if before.is_empty() {
            return None;
        }
        // On-premises servers keep their port, unless it is the SSH one.
        let authority = match port {
            Some(p) if !is_ssh && p != 443 => format!("{host}:{p}"),
            _ => host.clone(),
        };
        let web = format!("https://{authority}/{}", before.join("/"));
        return Some(Tracker { kind: TrackerKind::Azure, api: web.clone(), web, host });
    }
    let host = if host == "ssh.github.com" { "github.com".to_string() } else { host };
    if host.contains("github") && seg.len() == 2 {
        let web = format!("https://{host}/{}/{}", seg[0], seg[1]);
        let api = if host == "github.com" { format!("https://api.github.com/repos/{}/{}", seg[0], seg[1]) } else { format!("https://{host}/api/v3/repos/{}/{}", seg[0], seg[1]) };
        return Some(Tracker { kind: TrackerKind::GitHub, web, api, host });
    }
    if host.contains("gitlab") && seg.len() >= 2 {
        let web = format!("https://{host}/{}", seg.join("/"));
        let api = format!("https://{host}/api/v4/projects/{}", seg.join("%2F"));
        return Some(Tracker { kind: TrackerKind::GitLab, web, api, host });
    }
    None
}

/// A custom link pattern from the settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkPattern {
    /// A regular expression (`[A-Z]+-\d+`, `JIRA-(\d+)`…).
    pub pattern: String,
    /// The link, with `$0` (whole match), `$1`… or `${name}` replaced by the groups.
    pub url: String,
}

/// A work item reference found in a text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkItemLink {
    /// Byte range of the reference in the text.
    pub range: Range<usize>,
    pub url: String,
    /// The id (first capture group, or the whole match).
    pub id: String,
    /// The tracker of a built-in reference (`None` for custom patterns).
    pub kind: Option<TrackerKind>,
}

struct Rule {
    re: Regex,
    template: String,
    kind: Option<TrackerKind>,
    /// The reference must not follow a letter, digit, `_`, `&` or `#` (`a#1`, `&#38;`).
    standalone: bool,
}

/// Finds work item references in commit messages.
#[derive(Default)]
pub struct Linker {
    rules: Vec<Rule>,
    /// The trackers that `#` and `AB#` refer to.
    hash: Option<TrackerKind>,
    boards: Option<TrackerKind>,
}

/// `$` would start a group reference in a template.
fn literal(s: &str) -> String {
    s.replace('$', "$$")
}

impl Linker {
    /// Rules for the repository's trackers (the first one of each kind wins; `#` refers to the
    /// first tracker) and the custom patterns, which take precedence. Invalid custom patterns
    /// are returned as errors and skipped.
    pub fn new(trackers: &[Tracker], custom: &[LinkPattern]) -> (Linker, Vec<String>) {
        let mut linker = Linker::default();
        let mut errors = Vec::new();
        for p in custom {
            if p.pattern.trim().is_empty() || p.url.trim().is_empty() {
                continue;
            }
            match Regex::new(&p.pattern) {
                Ok(re) => linker.rules.push(Rule { re, template: p.url.trim().to_string(), kind: None, standalone: false }),
                Err(e) => errors.push(format!("{}: {e}", p.pattern)),
            }
        }
        let first = |k: TrackerKind| trackers.iter().find(|t| t.kind == k);
        let rule = |re: &str, template: String, kind: TrackerKind, standalone: bool| Rule { re: Regex::new(re).expect("built-in pattern"), template, kind: Some(kind), standalone };
        if let Some(t) = first(TrackerKind::Azure) {
            linker.rules.push(rule(r"\bAB#(\d+)\b", format!("{}/_workitems/edit/$1", literal(&t.web)), TrackerKind::Azure, false));
            linker.boards = Some(TrackerKind::Azure);
        }
        if let Some(t) = first(TrackerKind::GitHub) {
            let site = t.web.rsplitn(3, '/').nth(2).unwrap_or(&t.web).to_string();
            linker.rules.push(rule(r"\b([A-Za-z0-9][\w.-]*/[\w.-]+)#(\d+)\b", format!("{}/$1/issues/$2", literal(&site)), TrackerKind::GitHub, true));
            linker.rules.push(rule(r"\bGH-(\d+)\b", format!("{}/issues/$1", literal(&t.web)), TrackerKind::GitHub, false));
        }
        if let Some(t) = trackers.first() {
            let template = literal(&t.item_url("$1")).replace("$$1", "$1");
            linker.rules.push(rule(r"#(\d+)\b", template, t.kind, true));
            linker.hash = Some(t.kind);
        }
        (linker, errors)
    }

    /// The tracker a completion prefix (`#`, `AB#`) refers to.
    pub fn kind_for_prefix(&self, prefix: &str) -> Option<TrackerKind> {
        match prefix {
            "#" => self.hash,
            "AB#" => self.boards,
            _ => None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The references in `text`, in order and without overlaps (earlier rules and longer
    /// matches win).
    pub fn find(&self, text: &str) -> Vec<WorkItemLink> {
        let mut found: Vec<(usize, WorkItemLink)> = Vec::new();
        for (order, rule) in self.rules.iter().enumerate() {
            for caps in rule.re.captures_iter(text) {
                let m = caps.get(0).expect("group 0");
                if m.is_empty() {
                    continue;
                }
                if rule.standalone && text[..m.start()].chars().next_back().is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '&' | '#' | '/')) {
                    continue;
                }
                let mut url = String::new();
                caps.expand(&rule.template, &mut url);
                let id = caps.iter().skip(1).flatten().last().map_or(m.as_str(), |g| g.as_str()).to_string();
                found.push((order, WorkItemLink { range: m.range(), url, id, kind: rule.kind }));
            }
        }
        found.sort_by_key(|(order, l)| (l.range.start, *order, std::cmp::Reverse(l.range.end)));
        let mut out: Vec<WorkItemLink> = Vec::new();
        for (_, l) in found {
            if out.last().is_none_or(|prev| l.range.start >= prev.range.end) {
                out.push(l);
            }
        }
        out
    }
}

/// A work item offered by the completion of `#` / `AB#`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkItem {
    pub kind: TrackerKind,
    pub id: u64,
    pub title: String,
}

/// The ids referenced by recent commit messages (most recent first, each once), titled by the
/// subject of the newest commit that mentions them: the offline completion source.
pub fn recent_work_items<'a>(linker: &Linker, messages: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<WorkItem> {
    let mut out: Vec<WorkItem> = Vec::new();
    for (subject, body) in messages {
        for text in [subject, body] {
            for l in linker.find(text) {
                let (Some(kind), Ok(id)) = (l.kind, l.id.parse::<u64>()) else { continue };
                if !out.iter().any(|w| w.kind == kind && w.id == id) {
                    out.push(WorkItem { kind, id, title: subject.trim().to_string() });
                }
            }
        }
    }
    out
}

/// A completion being typed before the cursor: `#12` or `AB#` (byte offset of the reference,
/// its prefix and the digits typed so far).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionQuery {
    pub start: usize,
    pub prefix: &'static str,
    pub digits: String,
}

/// The reference being typed right before byte `cursor` of `text`, if any.
pub fn completion_query(text: &str, cursor: usize) -> Option<CompletionQuery> {
    let before = text.get(..cursor)?;
    let digits_start = before.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    let digits = &before[digits_start..];
    let head = before[..digits_start].strip_suffix('#')?;
    // A following letter or digit means the cursor is inside a word.
    if text[cursor..].chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let boundary = |s: &str| s.chars().next_back().is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '_' | '&' | '#' | '/')));
    if let Some(h) = head.strip_suffix("AB")
        && boundary(h)
    {
        return Some(CompletionQuery { start: h.len(), prefix: "AB#", digits: digits.to_string() });
    }
    boundary(head).then(|| CompletionQuery { start: head.len(), prefix: "#", digits: digits.to_string() })
}

/// The items matching a query: ids starting with the digits typed, at most `limit`.
pub fn complete<'a>(linker: &Linker, items: &'a [WorkItem], q: &CompletionQuery, limit: usize) -> Vec<&'a WorkItem> {
    let Some(kind) = linker.kind_for_prefix(q.prefix) else { return Vec::new() };
    items.iter().filter(|w| w.kind == kind && w.id.to_string().starts_with(&q.digits)).take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn web(url: &str) -> Option<(TrackerKind, String)> {
        tracker_from_remote(url).map(|t| (t.kind, t.web))
    }

    #[test]
    fn github_remotes() {
        let gh = Some((TrackerKind::GitHub, "https://github.com/owner/repo".to_string()));
        for url in [
            "https://github.com/owner/repo",
            "https://github.com/owner/repo.git",
            "https://github.com/owner/repo/",
            "https://user:token@github.com/owner/repo.git",
            "http://github.com/owner/repo",
            "git@github.com:owner/repo.git",
            "git@github.com:owner/repo",
            "ssh://git@github.com/owner/repo.git",
            "ssh://git@ssh.github.com:443/owner/repo.git",
            "git://github.com/owner/repo.git",
            "  https://GitHub.com/owner/repo.git\n",
        ] {
            assert_eq!(web(url), gh, "{url}");
        }
        let t = tracker_from_remote("git@github.com:owner/repo.git").unwrap();
        assert_eq!(t.api, "https://api.github.com/repos/owner/repo");
        assert_eq!(t.item_url("42"), "https://github.com/owner/repo/issues/42");
        let ghe = tracker_from_remote("https://github.contoso.com/team/app.git").unwrap();
        assert_eq!((ghe.web.as_str(), ghe.api.as_str()), ("https://github.contoso.com/team/app", "https://github.contoso.com/api/v3/repos/team/app"));
    }

    #[test]
    fn azure_remotes() {
        let dev = Some((TrackerKind::Azure, "https://dev.azure.com/org/Project".to_string()));
        for url in [
            "https://dev.azure.com/org/Project/_git/Repo",
            "https://org@dev.azure.com/org/Project/_git/Repo",
            "https://org@dev.azure.com/org/Project/_git/Repo/",
            "git@ssh.dev.azure.com:v3/org/Project/Repo",
            "ssh://git@ssh.dev.azure.com/v3/org/Project/Repo",
        ] {
            assert_eq!(web(url), dev, "{url}");
        }
        assert_eq!(web("https://dev.azure.com/org/_git/Repo"), Some((TrackerKind::Azure, "https://dev.azure.com/org/Repo".into())));
        assert_eq!(web("https://dev.azure.com/org/My%20Project/_git/My%20Repo"), Some((TrackerKind::Azure, "https://dev.azure.com/org/My%20Project".into())));
        let vs = Some((TrackerKind::Azure, "https://org.visualstudio.com/Project".to_string()));
        for url in [
            "https://org.visualstudio.com/Project/_git/Repo",
            "https://org.visualstudio.com/_git/Project",
            "org@vs-ssh.visualstudio.com:v3/org/Project/Repo",
            "ssh://org@vs-ssh.visualstudio.com:22/v3/org/Project/Repo",
        ] {
            assert_eq!(web(url), vs, "{url}");
        }
        assert_eq!(web("https://org.visualstudio.com/DefaultCollection/Project/_git/Repo"), Some((TrackerKind::Azure, "https://org.visualstudio.com/DefaultCollection/Project".into())));
        // Azure DevOps Server keeps its collection and port.
        assert_eq!(web("http://tfs.contoso.local:8080/tfs/DefaultCollection/Shop/_git/Shop"), Some((TrackerKind::Azure, "https://tfs.contoso.local:8080/tfs/DefaultCollection/Shop".into())));
        assert_eq!(web("ssh://tfs.contoso.local:22/tfs/DefaultCollection/Shop/_git/Shop"), Some((TrackerKind::Azure, "https://tfs.contoso.local/tfs/DefaultCollection/Shop".into())));
        let t = tracker_from_remote("git@ssh.dev.azure.com:v3/org/Project/Repo").unwrap();
        assert_eq!(t.item_url("1234"), "https://dev.azure.com/org/Project/_workitems/edit/1234");
        assert_eq!(t.host, "dev.azure.com");
    }

    #[test]
    fn other_remotes() {
        assert_eq!(web("git@gitlab.com:group/sub/repo.git"), Some((TrackerKind::GitLab, "https://gitlab.com/group/sub/repo".into())));
        assert_eq!(tracker_from_remote("https://gitlab.com/group/repo").unwrap().item_url("7"), "https://gitlab.com/group/repo/-/issues/7");
        for url in ["/srv/git/repo.git", "file:///srv/git/repo.git", "C:\\repos\\shop", "https://bitbucket.org/team/repo.git", "", "https://github.com/owner"] {
            assert_eq!(web(url), None, "{url}");
        }
    }

    fn linker(remotes: &[&str], custom: &[(&str, &str)]) -> Linker {
        let trackers: Vec<Tracker> = remotes.iter().filter_map(|r| tracker_from_remote(r)).collect();
        let custom: Vec<LinkPattern> = custom.iter().map(|(p, u)| LinkPattern { pattern: p.to_string(), url: u.to_string() }).collect();
        let (l, errors) = Linker::new(&trackers, &custom);
        assert!(errors.is_empty(), "{errors:?}");
        l
    }

    fn links(l: &Linker, text: &str) -> Vec<(String, String)> {
        l.find(text).into_iter().map(|w| (text[w.range].to_string(), w.url)).collect()
    }

    #[test]
    fn github_references() {
        let l = linker(&["git@github.com:owner/repo.git"], &[]);
        assert_eq!(
            links(&l, "Fix #12 and GH-7 (see other/lib#3), not a#1, &#38; or ##5"),
            vec![
                ("#12".into(), "https://github.com/owner/repo/issues/12".into()),
                ("GH-7".into(), "https://github.com/owner/repo/issues/7".into()),
                ("other/lib#3".into(), "https://github.com/other/lib/issues/3".into()),
            ]
        );
        assert_eq!(links(&l, "#1"), vec![("#1".into(), "https://github.com/owner/repo/issues/1".into())]);
        // AB# needs an Azure remote.
        assert!(links(&l, "AB#1 x#2 #12abc").is_empty());
        assert_eq!(l.kind_for_prefix("#"), Some(TrackerKind::GitHub));
        assert_eq!(l.kind_for_prefix("AB#"), None);
    }

    #[test]
    fn azure_references() {
        let l = linker(&["https://dev.azure.com/org/Project/_git/Repo"], &[]);
        assert_eq!(
            links(&l, "AB#1234: merge (#56)\nFixes AB#7, not XAB#8"),
            vec![
                ("AB#1234".into(), "https://dev.azure.com/org/Project/_workitems/edit/1234".into()),
                ("#56".into(), "https://dev.azure.com/org/Project/_workitems/edit/56".into()),
                ("AB#7".into(), "https://dev.azure.com/org/Project/_workitems/edit/7".into()),
            ]
        );
        assert_eq!(l.kind_for_prefix("AB#"), Some(TrackerKind::Azure));
        // GitHub repository linked to Azure Boards: # is the issue, AB# the work item.
        let both = linker(&["https://github.com/owner/repo", "https://dev.azure.com/org/Project/_git/Repo"], &[]);
        assert_eq!(
            links(&both, "AB#9 #9"),
            vec![("AB#9".into(), "https://dev.azure.com/org/Project/_workitems/edit/9".into()), ("#9".into(), "https://github.com/owner/repo/issues/9".into())]
        );
    }

    #[test]
    fn custom_patterns_win_and_bad_ones_are_reported() {
        let l = linker(&["https://github.com/owner/repo"], &[(r"\b(SHOP-\d+)\b", "https://jira.contoso.com/browse/$1"), (r"#(\d{4,})", "https://tracker.local/$1")]);
        assert_eq!(
            links(&l, "SHOP-12 #1234 #5"),
            vec![
                ("SHOP-12".into(), "https://jira.contoso.com/browse/SHOP-12".into()),
                ("#1234".into(), "https://tracker.local/1234".into()),
                ("#5".into(), "https://github.com/owner/repo/issues/5".into()),
            ]
        );
        let custom = [LinkPattern { pattern: "([".into(), url: "x".into() }, LinkPattern { pattern: "TICKET-(?P<n>\\d+)".into(), url: "https://t/${n}?a=$$".into() }];
        let (l, errors) = Linker::new(&[], &custom);
        assert_eq!(errors.len(), 1);
        assert_eq!(links(&l, "TICKET-3"), vec![("TICKET-3".into(), "https://t/3?a=$".into())]);
        assert!(links(&l, "#3").is_empty());
        let (empty, _) = Linker::new(&[], &[]);
        assert!(empty.is_empty() && empty.find("#1 AB#2").is_empty());
    }

    #[test]
    fn utf8_text_keeps_byte_ranges() {
        let l = linker(&["https://github.com/o/r"], &[]);
        let text = "Corrige cálculo — ver #8 “ñ”";
        let found = l.find(text);
        assert_eq!(&text[found[0].range.clone()], "#8");
    }

    #[test]
    fn completion_queries() {
        let q = |text: &str| completion_query(text, text.len());
        assert_eq!(q("Fix #"), Some(CompletionQuery { start: 4, prefix: "#", digits: String::new() }));
        assert_eq!(q("Fix #12"), Some(CompletionQuery { start: 4, prefix: "#", digits: "12".into() }));
        assert_eq!(q("AB#"), Some(CompletionQuery { start: 0, prefix: "AB#", digits: String::new() }));
        assert_eq!(q("(AB#7"), Some(CompletionQuery { start: 1, prefix: "AB#", digits: "7".into() }));
        assert_eq!(q("#"), Some(CompletionQuery { start: 0, prefix: "#", digits: String::new() }));
        for none in ["Fix", "a#1", "C#", "&#", "##", "Fix #1 ", "XAB#2", "owner/repo#"] {
            assert_eq!(q(none), None, "{none}");
        }
        // Inside a word, or at a byte that is not a char boundary.
        assert_eq!(completion_query("#12x", 3), None);
        assert_eq!(completion_query("ñ#", 1), None);
        assert_eq!(completion_query("Fix #1 now", 6).map(|q| q.digits), Some("1".into()));
    }

    #[test]
    fn offline_items_come_from_recent_messages() {
        let l = linker(&["https://dev.azure.com/org/P/_git/R"], &[]);
        let log = [("Add coupons AB#12", ""), ("Fix rounding", "Related to #12 and AB#30"), ("Old #5", "")];
        let items = recent_work_items(&l, log);
        assert_eq!(items.iter().map(|w| (w.id, w.title.as_str())).collect::<Vec<_>>(), vec![(12, "Add coupons AB#12"), (30, "Fix rounding"), (5, "Old #5")]);
        let q = completion_query("AB#", 3).unwrap();
        assert_eq!(complete(&l, &items, &q, 10).len(), 3);
        let q = completion_query("#3", 2).unwrap();
        assert_eq!(complete(&l, &items, &q, 10).iter().map(|w| w.id).collect::<Vec<_>>(), vec![30]);
        let gh = linker(&["https://github.com/o/r"], &[]);
        assert!(complete(&gh, &items, &completion_query("AB#", 3).unwrap(), 10).is_empty());
    }
}
