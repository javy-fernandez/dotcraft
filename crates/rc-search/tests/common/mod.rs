//! A synthetic .NET tree for timing tests and benches.

const WORDS: &[&str] = &[
    "Order", "Customer", "Invoice", "Product", "Payment", "Shipping", "Account", "Report", "Catalog", "Basket", "Discount",
    "Warehouse", "Supplier", "Employee", "Session", "Token", "Audit", "Notification", "Search", "Pricing",
];
const SUFFIXES: &[&str] = &["Service", "Controller", "Repository", "Model", "Validator", "Handler", "Tests", "Dto", "Mapper", "Factory"];
const FOLDERS: &[&str] = &["Services", "Controllers", "Data/Repositories", "Models", "Validation", "Handlers", "Tests/Unit", "Contracts", "Mapping", "Infrastructure/Factories"];
const EXTS: &[&str] = &["cs", "cs", "cs", "cs", "json", "md", "xml", "csproj"];

/// `n` distinct relative paths spread over projects and folders.
pub fn synthetic_paths(n: usize) -> Vec<String> {
    (0..n)
        .map(|i| {
            let project = format!("src/Shop.Module{:02}", i % 60);
            let folder = FOLDERS[(i / 7) % FOLDERS.len()];
            let a = WORDS[i % WORDS.len()];
            let b = WORDS[(i / WORDS.len()) % WORDS.len()];
            let s = SUFFIXES[(i / 3) % SUFFIXES.len()];
            let ext = EXTS[(i / 11) % EXTS.len()];
            format!("{project}/{folder}/{a}{b}{s}{i}.{ext}")
        })
        .collect()
}
