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
