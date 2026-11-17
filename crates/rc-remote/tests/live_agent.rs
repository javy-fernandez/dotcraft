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
    let mut client = Client::new(
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
    let mut client = Client::new(
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
    let error = client
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
        .unwrap_err();
    assert!(error.to_string().contains("cancelled"));
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
    assert!(matches!(
        client.request(Operation::Hello).await.unwrap(),
        Value::Hello { .. }
    ));
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
    let mut client = Client::new(
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
