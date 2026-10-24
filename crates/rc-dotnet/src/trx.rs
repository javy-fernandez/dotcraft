//! Parser for VSTest TRX result files (xUnit, NUnit and MSTest all write the same schema).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TestOutcome {
    Passed,
    Failed,
    Skipped,
}

impl TestOutcome {
    /// Maps TRX / console outcomes (`NotExecuted`, `Timeout`, `Error`…) to the three kinds.
    pub fn from_trx(s: &str) -> TestOutcome {
        match s.to_ascii_lowercase().as_str() {
            "passed" | "passedbutrunaborted" | "warning" => TestOutcome::Passed,
            "notexecuted" | "skipped" | "inconclusive" | "pending" | "notrunnable" | "disconnected" => TestOutcome::Skipped,
            _ => TestOutcome::Failed,
        }
    }
}

/// One frame of a .NET stack trace (`at Ns.C.M() in /p/File.cs:line 12`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StackFrame {
    pub text: String,
    pub file: Option<PathBuf>,
    pub line: Option<u32>,
}

/// The result of one test (one row per data case of a theory).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestResult {
    /// `Namespace.Class.Method`, without data arguments.
    pub fully_qualified_name: String,
    /// The name the framework shows (`Method(9,0)`, `Ns.Class.Method(coupon: "x")`…).
    pub display_name: String,
    pub class_name: String,
    pub method_name: String,
    pub outcome: TestOutcome,
    pub duration: Duration,
    pub error_message: Option<String>,
    pub stack_trace: Vec<StackFrame>,
    pub stdout: String,
    pub stderr: String,
    /// The test assembly.
    pub source: Option<PathBuf>,
}

/// Reads and parses a TRX file.
pub fn load_trx(path: &Path) -> anyhow::Result<Vec<TestResult>> {
    parse_trx(&std::fs::read_to_string(path)?)
}

/// Parses TRX text into results, in the order of `<Results>`.
pub fn parse_trx(text: &str) -> anyhow::Result<Vec<TestResult>> {
    let doc = roxmltree::Document::parse(text.trim_start_matches('\u{feff}'))?;
    struct Def<'a> {
        class: &'a str,
        method: &'a str,
        source: Option<&'a str>,
    }
    let mut defs: HashMap<&str, Def> = HashMap::new();
    for t in doc.descendants().filter(|n| n.has_tag_name("UnitTest")) {
        let Some(id) = t.attribute("id") else { continue };
        let tm = t.children().find(|c| c.has_tag_name("TestMethod"));
        defs.insert(
            id,
            Def {
                class: tm.and_then(|m| m.attribute("className")).unwrap_or(""),
                method: tm.and_then(|m| m.attribute("name")).unwrap_or(""),
                source: tm.and_then(|m| m.attribute("codeBase")).or(t.attribute("storage")),
            },
        );
    }
    let mut out = Vec::new();
    let Some(results) = doc.descendants().find(|n| n.has_tag_name("Results")) else { return Ok(out) };
    for r in results.children().filter(|n| n.has_tag_name("UnitTestResult")) {
        let display = r.attribute("testName").unwrap_or("").to_string();
        let def = r.attribute("testId").and_then(|id| defs.get(id));
        let (class_name, method_name) = match def {
            // NUnit writes data arguments into the method name: `M(9,0)`.
            Some(d) if !d.method.is_empty() => (d.class.to_string(), d.method.split('(').next().unwrap_or("").trim_end().to_string()),
            _ => split_test_name(&display),
        };
        // NUnit/xUnit may write the class with the assembly ("Ns.C, Asm").
        let class_name = class_name.split(',').next().unwrap_or("").trim().to_string();
        let output = r.children().find(|c| c.has_tag_name("Output"));
        let child_text = |parent: Option<roxmltree::Node>, tag: &str| parent.and_then(|o| o.descendants().find(|c| c.has_tag_name(tag))).and_then(|n| n.text()).map(str::to_string);
        let error = output.and_then(|o| o.children().find(|c| c.has_tag_name("ErrorInfo")));
        let mut error_message = child_text(error, "Message").map(|s| s.trim().to_string());
        let stack = child_text(error, "StackTrace").map(|s| parse_stack_trace(&s)).unwrap_or_default();
        let outcome = TestOutcome::from_trx(r.attribute("outcome").unwrap_or(""));
        if error_message.is_none() && outcome == TestOutcome::Skipped {
            // Skip reasons live in <Output><TextMessages><Message>.
            error_message = output
                .and_then(|o| o.children().find(|c| c.has_tag_name("TextMessages")))
                .and_then(|t| t.children().find(|c| c.has_tag_name("Message")))
                .and_then(|m| m.text())
                .map(|s| s.trim().to_string());
        }
        out.push(TestResult {
            fully_qualified_name: if class_name.is_empty() { method_name.clone() } else { format!("{class_name}.{method_name}") },
            display_name: display,
            class_name,
            method_name,
            outcome,
            duration: r.attribute("duration").and_then(parse_duration).unwrap_or_default(),
            error_message,
            stack_trace: stack,
            stdout: child_text(output, "StdOut").unwrap_or_default(),
            stderr: child_text(output, "StdErr").unwrap_or_default(),
            source: def.and_then(|d| d.source).map(PathBuf::from),
        });
    }
    Ok(out)
}

/// `Ns.Class.Method(args)` → (`Ns.Class`, `Method`). Dots inside the arguments are ignored.
pub fn split_test_name(name: &str) -> (String, String) {
    let base = name.find('(').map_or(name, |p| &name[..p]).trim_end();
    match base.rfind('.') {
        Some(dot) => (base[..dot].to_string(), base[dot + 1..].to_string()),
        None => (String::new(), base.to_string()),
    }
}

/// `00:00:01.2500000` (hh:mm:ss.fffffff) → duration.
pub fn parse_duration(s: &str) -> Option<Duration> {
    let mut parts = s.trim().split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let sec: f64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(Duration::from_secs_f64(h * 3600.0 + m * 60.0 + sec))
}

/// Splits a stack trace into frames, extracting `in <file>:line <n>`.
pub fn parse_stack_trace(text: &str) -> Vec<StackFrame> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            let (file, line) = match l.rfind(" in ") {
                Some(i) => {
                    let loc = &l[i + 4..];
                    match loc.rfind(":line ") {
                        Some(j) => (Some(PathBuf::from(&loc[..j])), loc[j + 6..].trim().parse().ok()),
                        None => (None, None),
                    }
                }
                None => (None, None),
            };
            StackFrame { text: l.to_string(), file, line }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRX: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<TestRun id="1" name="run" xmlns="http://microsoft.com/schemas/VisualStudio/TeamTest/2010">
  <Results>
    <UnitTestResult testId="a" testName="Shop.Tests.CalcTests.Adds" duration="00:00:00.0125000" outcome="Passed">
      <Output><StdOut>hello
world</StdOut></Output>
    </UnitTestResult>
    <UnitTestResult testId="b" testName="Shop.Tests.CalcTests.Divides(x: 1.5, y: 0)" duration="00:00:01.5000000" outcome="Failed">
      <Output>
        <ErrorInfo>
          <Message>Assert.Equal() Failure: Values differ
Expected: 1
Actual:   2</Message>
          <StackTrace>   at Shop.Tests.CalcTests.Divides(Double x, Double y) in /w/tests/CalcTests.cs:line 42
   at System.RuntimeMethodHandle.InvokeMethod(Object target, Void** arguments, Signature sig, Boolean isConstructor)
   at C.M() in C:\src\T.cs:line 7</StackTrace>
        </ErrorInfo>
      </Output>
    </UnitTestResult>
    <UnitTestResult testId="c" testName="Ignored" outcome="NotExecuted">
      <Output><TextMessages><Message>Not today</Message></TextMessages></Output>
    </UnitTestResult>
  </Results>
  <TestDefinitions>
    <UnitTest name="Adds" storage="/w/tests/bin/t.dll" id="a"><TestMethod codeBase="/w/tests/bin/T.dll" className="Shop.Tests.CalcTests" name="Adds" /></UnitTest>
    <UnitTest name="Divides" storage="/w/tests/bin/t.dll" id="b"><TestMethod codeBase="/w/tests/bin/T.dll" className="Shop.Tests.CalcTests, T, Version=1.0.0.0" name="Divides" /></UnitTest>
    <UnitTest name="Ignored" storage="/w/tests/bin/t.dll" id="c"><TestMethod codeBase="/w/tests/bin/T.dll" className="Shop.Tests.Other" name="Ignored(9,0)" /></UnitTest>
  </TestDefinitions>
</TestRun>"#;

    #[test]
    fn parses_passed_failed_and_skipped_results() {
        let r = parse_trx(TRX).unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].fully_qualified_name, "Shop.Tests.CalcTests.Adds");
        assert_eq!(r[0].outcome, TestOutcome::Passed);
        assert_eq!(r[0].duration, Duration::from_micros(12500));
        assert_eq!(r[0].stdout, "hello\nworld");
        assert_eq!(r[0].source, Some(PathBuf::from("/w/tests/bin/T.dll")));

        let f = &r[1];
        assert_eq!((f.class_name.as_str(), f.method_name.as_str()), ("Shop.Tests.CalcTests", "Divides"));
        assert_eq!(f.display_name, "Shop.Tests.CalcTests.Divides(x: 1.5, y: 0)");
        assert_eq!(f.outcome, TestOutcome::Failed);
        assert!(f.error_message.as_deref().unwrap().starts_with("Assert.Equal() Failure"));
        assert_eq!(f.stack_trace.len(), 3);
        assert_eq!(f.stack_trace[0].file, Some(PathBuf::from("/w/tests/CalcTests.cs")));
        assert_eq!(f.stack_trace[0].line, Some(42));
        assert_eq!(f.stack_trace[1].file, None);
        assert_eq!(f.stack_trace[2].file, Some(PathBuf::from(r"C:\src\T.cs")));

        assert_eq!(r[2].outcome, TestOutcome::Skipped);
        assert_eq!(r[2].error_message.as_deref(), Some("Not today"));
        assert_eq!(r[2].fully_qualified_name, "Shop.Tests.Other.Ignored");
    }

    #[test]
    fn splits_names_and_durations() {
        assert_eq!(split_test_name("A.B.C(1.5, \"x.y\")"), ("A.B".to_string(), "C".to_string()));
        assert_eq!(split_test_name("Method"), (String::new(), "Method".to_string()));
        assert_eq!(parse_duration("01:02:03.5"), Some(Duration::from_secs_f64(3723.5)));
        assert_eq!(parse_duration("bad"), None);
        assert_eq!(TestOutcome::from_trx("Timeout"), TestOutcome::Failed);
        assert_eq!(TestOutcome::from_trx("Inconclusive"), TestOutcome::Skipped);
    }
}
