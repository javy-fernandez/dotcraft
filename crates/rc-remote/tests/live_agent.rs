use rc_remote::{Client, Operation, Value, decode_data};
use std::process::Stdio;
use tokio::process::Command;

#[tokio::test]
async fn local_agent_binary_lists_and_reads_a_workspace() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Program.cs"), "class Program {}\n").unwrap();
    let mut process = Command::new(env!("CARGO_BIN_EXE_dotcraft-remote-agent"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let client = Client::new(
        process.stdout.take().unwrap(),
        process.stdin.take().unwrap(),
    );
    client.handshake().await.unwrap();
    let Value::Entries { entries } = client
        .request(Operation::List {
            path: dir.path().display().to_string(),
        })
        .await
        .unwrap()
    else {
        panic!("directory response")
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "Program.cs");
    let Value::Data { data } = client
        .request(Operation::Read {
            path: dir.path().join("Program.cs").display().to_string(),
        })
        .await
        .unwrap()
    else {
        panic!("file response")
    };
    assert_eq!(decode_data(&data).unwrap(), b"class Program {}\n");
    drop(client);
    assert!(process.wait().await.unwrap().success());
}

#[cfg(unix)]
#[tokio::test]
async fn cancelled_command_keeps_the_agent_connection_usable() {
    let mut process = Command::new(env!("CARGO_BIN_EXE_dotcraft-remote-agent"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let client = Client::new(
        process.stdout.take().unwrap(),
        process.stdin.take().unwrap(),
    );
    client.handshake().await.unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let _ = tx.send(());
    });
    let start = std::time::Instant::now();
    let cancelled = client
        .request_cancellable(
            Operation::Run {
                program: "sleep".into(),
                args: vec!["10".into()],
                cwd: std::env::temp_dir().display().to_string(),
                env: vec![],
                timeout_ms: 20_000,
            },
            rx,
        )
        .await
        .unwrap();
    assert!(cancelled.is_none());
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
    assert!(matches!(
        client.request(Operation::Hello).await.unwrap(),
        Value::Hello { .. }
    ));
    drop(client);
    assert!(process.wait().await.unwrap().success());
}

/// A long command does not hold up browsing: other requests are answered meanwhile.
#[cfg(unix)]
#[tokio::test]
async fn requests_run_side_by_side_with_a_long_command() {
    let mut process = Command::new(env!("CARGO_BIN_EXE_dotcraft-remote-agent"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let client = Client::new(
        process.stdout.take().unwrap(),
        process.stdin.take().unwrap(),
    );
    client.handshake().await.unwrap();
    let build = {
        let client = client.clone();
        tokio::spawn(async move {
            client
                .request(Operation::Run {
                    program: "sh".into(),
                    args: vec!["-c".into(), "sleep 1; echo built".into()],
                    cwd: std::env::temp_dir().display().to_string(),
                    env: vec![],
                    timeout_ms: 20_000,
                })
                .await
        })
    };
    let start = std::time::Instant::now();
    let dir = tempfile::tempdir().unwrap();
    for _ in 0..3 {
        let listed = client
            .request(Operation::List {
                path: dir.path().display().to_string(),
            })
            .await
            .unwrap();
        assert!(matches!(listed, Value::Entries { .. }));
    }
    assert!(start.elapsed() < std::time::Duration::from_millis(800));
    let Value::Exit { stdout, .. } = build.await.unwrap().unwrap() else {
        panic!("process response")
    };
    assert_eq!(decode_data(&stdout).unwrap(), b"built\n");
    drop(client);
    assert!(process.wait().await.unwrap().success());
}

/// Needs the .NET SDK: a real build through the agent finishes when `dotnet` does, although
/// MSBuild nodes and build servers it leaves running keep its output pipes open.
#[cfg(unix)]
#[ignore]
#[tokio::test]
async fn a_real_dotnet_build_returns_when_dotnet_exits() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("App.csproj"),
        "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("Program.cs"), "System.Console.WriteLine(\"hi\");\n").unwrap();
    let mut process = Command::new(env!("CARGO_BIN_EXE_dotcraft-remote-agent"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let client = Client::new(
        process.stdout.take().unwrap(),
        process.stdin.take().unwrap(),
    );
    client.handshake().await.unwrap();
    for _ in 0..2 {
        let start = std::time::Instant::now();
        let Value::Exit { code, stdout, stderr } = client
            .request(Operation::Run {
                program: "dotnet".into(),
                args: vec!["build".into(), "App.csproj".into()],
                cwd: dir.path().display().to_string(),
                env: vec![
                    ("DOTNET_NOLOGO".into(), "1".into()),
                    ("DOTNET_CLI_UI_LANGUAGE".into(), "en".into()),
                ],
                timeout_ms: 300_000,
            })
            .await
            .unwrap()
        else {
            panic!("process response")
        };
        let stdout = String::from_utf8(decode_data(&stdout).unwrap()).unwrap();
        let stderr = String::from_utf8(decode_data(&stderr).unwrap()).unwrap();
        assert_eq!(code, Some(0), "{stdout}{stderr}");
        assert!(stdout.contains("Build succeeded"), "{stdout}");
        eprintln!("dotnet build through the agent: {:?}", start.elapsed());
    }
    drop(client);
    assert!(process.wait().await.unwrap().success());
}

#[cfg(unix)]
#[tokio::test]
async fn local_agent_runs_dotnet_discovery_build_and_test_commands() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let dotnet = dir.path().join("dotnet");
    std::fs::write(&dotnet, "#!/bin/sh\ncase \"$1\" in\n  --info) printf 'SDK info\\n';;\n  --list-sdks) printf '10.0.100 [/sdk]\\n';;\n  build) printf 'Build succeeded\\n';;\n  test) printf 'Test Run Successful\\n';;\nesac\n").unwrap();
    std::fs::set_permissions(&dotnet, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut process = Command::new(env!("CARGO_BIN_EXE_dotcraft-remote-agent"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let client = Client::new(
        process.stdout.take().unwrap(),
        process.stdin.take().unwrap(),
    );
    client.handshake().await.unwrap();
    for (args, expected) in [
        (vec!["--info"], "SDK info"),
        (vec!["--list-sdks"], "10.0.100 [/sdk]"),
        (vec!["build", "App.csproj"], "Build succeeded"),
        (vec!["test", "App.csproj"], "Test Run Successful"),
    ] {
        let Value::Exit { code, stdout, .. } = client
            .request(Operation::Run {
                program: dotnet.display().to_string(),
                args: args.into_iter().map(str::to_string).collect(),
                cwd: dir.path().display().to_string(),
                env: vec![],
                timeout_ms: 5_000,
            })
            .await
            .unwrap()
        else {
            panic!("process response")
        };
        assert_eq!(code, Some(0));
        assert!(
            String::from_utf8(decode_data(&stdout).unwrap())
                .unwrap()
                .contains(expected)
        );
    }
    drop(client);
    assert!(process.wait().await.unwrap().success());
}

/// The release archive layout (`dotcraft-remote-agent` at the top, as the release workflows
/// pack it) deploys through the same steps as SSH, starts and answers the handshake.
#[cfg(unix)]
#[tokio::test]
async fn the_deployed_agent_binary_starts_and_answers() {
    use rc_remote::deploy::{AGENT_FILE, Fetch, LocalShell, Platform, Release, ensure_agent, sha256_hex};
    struct Files(Vec<(String, Vec<u8>)>);
    impl Fetch for Files {
        async fn get(&self, url: &str) -> anyhow::Result<Vec<u8>> {
            self.0
                .iter()
                .find(|(u, _)| u == url)
                .map(|(_, b)| b.clone())
                .ok_or_else(|| anyhow::anyhow!("404"))
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let (stage, home) = (dir.path().join("stage"), dir.path().join("home"));
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_dotcraft-remote-agent"), stage.join(AGENT_FILE)).unwrap();
    let archive = dir.path().join("agent.tar.gz");
    let packed = std::process::Command::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(&stage)
        .arg(AGENT_FILE)
        .status()
        .unwrap();
    assert!(packed.success());
    let bytes = std::fs::read(&archive).unwrap();
    let uname = std::process::Command::new("uname").arg("-sm").output().unwrap();
    let platform = Platform::from_uname(&String::from_utf8_lossy(&uname.stdout)).unwrap();
    // An address the "remote" cannot download from: DotCraft copies the archive over.
    let release = Release {
        version: "0.0.0-test".into(),
        base_url: format!("file://{}", dir.path().join("nowhere").display()),
    };
    let files = Files(vec![
        (
            release.checksum_url(platform),
            format!("{}  {}\n", sha256_hex(&bytes), release.asset(platform)).into_bytes(),
        ),
        (release.url(platform), bytes),
    ]);
    let shell = LocalShell { home: home.clone(), like_ssh: true };
    let agent = ensure_agent(&shell, &files, &release, &|_| {}).await.unwrap();
    let mut process = Command::new(home.join(agent.strip_prefix("~/").unwrap()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let client = Client::new(process.stdout.take().unwrap(), process.stdin.take().unwrap());
    client.handshake().await.unwrap();
    drop(client);
    assert!(process.wait().await.unwrap().success());
}
