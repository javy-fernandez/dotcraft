//! Lightweight live XAML design preview from the unsaved editor buffer.

use rc_core::{Inbox, Runtime, Waker};
use rc_kit::i18n::tr;
use std::path::{Path, PathBuf};

const MAX_BYTES: usize = 256 * 1024;
const MAX_NODES: usize = 500;
const MAX_DEPTH: usize = 24;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Element {
    pub tag: String,
    pub text: String,
    pub placeholder: String,
    pub orientation: String,
    pub children: Vec<Element>,
}

struct ResultModel {
    path: PathBuf,
    version: u64,
    result: Result<Element, String>,
}

pub struct XamlPreview {
    pub path: Option<PathBuf>,
    pub model: Option<Element>,
    pub error: Option<String>,
    pub loading: bool,
    version: Option<u64>,
    inbox: Inbox<ResultModel>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl XamlPreview {
    pub fn new(waker: Waker) -> Self {
        Self { path: None, model: None, error: None, loading: false, version: None,
            inbox: Inbox::new(waker), task: None }
    }

    pub fn observe<T: ToString + Send + 'static>(&mut self, path: &Path, version: u64, snapshot: T) {
        if self.path.as_deref() == Some(path) && self.version == Some(version) { return; }
        if let Some(task) = self.task.take() { task.abort(); }
        if self.path.as_deref() != Some(path) { self.model = None; self.error = None; }
        self.path = Some(path.to_path_buf());
        self.version = Some(version);
        self.loading = true;
        let out = self.inbox.sender();
        let path = path.to_path_buf();
        self.task = Some(Runtime::spawn(async move {
            let parsed = tokio::task::spawn_blocking(move || {
                let source = snapshot.to_string();
                if source.len() > MAX_BYTES { return Err(tr("XAML preview is limited to 256 KB").to_owned()); }
                parse(&source)
            }).await.unwrap_or_else(|error| Err(error.to_string()));
            out.send(ResultModel { path, version, result: parsed });
        }));
    }

    pub fn poll(&mut self) {
        for message in self.inbox.drain() {
            if self.path.as_ref() != Some(&message.path) || self.version != Some(message.version) { continue; }
            self.loading = false;
            self.task.take();
            match message.result {
                Ok(model) => { self.model = Some(model); self.error = None; }
                Err(error) => { self.model = None; self.error = Some(error); }
            }
        }
    }

    pub fn clear(&mut self) {
        if let Some(task) = self.task.take() { task.abort(); }
        self.inbox.drain();
        self.path = None;
        self.version = None;
        self.model = None;
        self.error = None;
        self.loading = false;
    }
}

fn parse(source: &str) -> Result<Element, String> {
    let doc = roxmltree::Document::parse(source).map_err(|error| error.to_string())?;
    let mut count = 0;
    build(doc.root_element(), 0, &mut count)
}

fn build(node: roxmltree::Node<'_, '_>, depth: usize, count: &mut usize) -> Result<Element, String> {
    if depth > MAX_DEPTH || *count >= MAX_NODES { return Err(tr("XAML preview has too many nested elements").into()); }
    *count += 1;
    let attr = |name: &str| node.attribute(name).unwrap_or("").to_owned();
    let tag = node.tag_name().name().to_owned();
    let text = [attr("Text"), attr("Content")].into_iter().find(|value| !value.is_empty())
        .unwrap_or_else(|| node.children().filter(|child| child.is_text()).filter_map(|child| child.text()).collect::<String>().trim().to_owned());
    let children = node.children().filter(|child| child.is_element()).map(|child| build(child, depth + 1, count)).collect::<Result<Vec<_>, _>>()?;
    let placeholder = [attr("Placeholder"), attr("Watermark")].into_iter().find(|value| !value.is_empty()).unwrap_or_default();
    Ok(Element { tag, text, placeholder, orientation: attr("Orientation"), children })
}

pub fn is_xaml(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| extension.eq_ignore_ascii_case("xaml") || extension.eq_ignore_ascii_case("axaml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_maui_and_avalonia_controls_without_namespaces_in_labels() {
        let maui = parse(r#"<ContentPage xmlns="http://schemas.microsoft.com/dotnet/2021/maui"><VerticalStackLayout><Label Text="Welcome"/><Button Text="Continue"/></VerticalStackLayout></ContentPage>"#).unwrap();
        assert_eq!(maui.tag, "ContentPage");
        assert_eq!(maui.children[0].children[0].text, "Welcome");
        let avalonia = parse(r#"<Window xmlns="https://github.com/avaloniaui"><StackPanel Orientation="Horizontal"><TextBlock Text="Name"/><TextBox Watermark="Enter name"/></StackPanel></Window>"#).unwrap();
        assert_eq!(avalonia.children[0].orientation, "Horizontal");
        assert!(is_xaml(Path::new("View.axaml")));
    }

    #[test]
    fn malformed_and_excessively_nested_markup_is_rejected() {
        assert!(parse("<Grid><Label></Grid>").is_err());
        let nested = format!("{}{}", "<Grid>".repeat(MAX_DEPTH + 2), "</Grid>".repeat(MAX_DEPTH + 2));
        assert!(parse(&nested).is_err());
    }
}
