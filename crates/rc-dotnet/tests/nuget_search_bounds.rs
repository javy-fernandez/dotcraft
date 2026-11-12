use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;

use rc_dotnet::nuget::NugetClient;
use rc_dotnet::ProjectInfo;

#[derive(Clone, Copy)]
enum ResponseSize { Normal, OversizedHeader, OversizedStream }

fn fake_feed(package_id: String, size: ResponseSize) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let base = format!("http://{address}");
    let index = format!("{base}/v3/index.json");
    let server_base = base.clone();
    let handle = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0; 8192];
            let read = stream.read(&mut buf).unwrap();
            let line = String::from_utf8_lossy(&buf[..read]).lines().next().unwrap_or("").to_string();
            requests.push(line.clone());
            if line.contains("/v3/index.json") {
                let body = serde_json::json!({"resources":[{"@id":format!("{server_base}/query"),"@type":"SearchQueryService/3.5.0"}]}).to_string();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            } else {
                match size {
                    ResponseSize::OversizedHeader => {
                        write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", 4 * 1024 * 1024 + 1).unwrap();
                    }
                    ResponseSize::OversizedStream => {
                        write!(stream, "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:X}\r\n", 4 * 1024 * 1024 + 1).unwrap();
                        let _ = stream.write_all(&vec![b'x'; 4 * 1024 * 1024 + 1]);
                    }
                    ResponseSize::Normal => {
                        let body = serde_json::json!({"totalHits":1,"data":[{"id":package_id,"version":"6.6.2","description":"Shop fixture package","versions":[{"version":"6.6.2"}]}]}).to_string();
                        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                    }
                }
            }
        }
        requests
    });
    (index, handle)
}

#[tokio::test]
async fn shop_package_search_pages_are_bounded_and_url_encoded() {
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src/Shop.Api/Shop.Api.csproj");
    let id = ProjectInfo::load(&project).unwrap().packages[0].name.clone();
    assert_eq!(id, "Swashbuckle.AspNetCore");
    let invalid = NugetClient::with_source("http://127.0.0.1:1/index.json").unwrap();
    for (query, skip, take) in [("a", 0, 0), ("a", 3001, 10), ("a", 0, 1001), ("x\n", 0, 10)] {
        assert!(invalid.search(query, false, skip, take).await.is_err());
    }
    assert!(invalid.search(&"x".repeat(257), false, 0, 10).await.is_err());
    let (index, handle) = fake_feed(id.clone(), ResponseSize::Normal);
    let client = NugetClient::with_source(&index).unwrap();
    let results = client.search(&format!("{id} & tools"), false, 0, 10).await.unwrap();
    assert_eq!(results.total_hits, 1);
    assert_eq!(results.packages[0].id, id);
    assert_eq!(results.packages[0].version, "6.6.2");
    let requests = handle.join().unwrap();
    assert!(requests[1].contains("q=Swashbuckle.AspNetCore+%26+tools"));
    assert!(requests[1].contains("take=10"));
    assert!(requests[1].contains("semVerLevel=2.0.0"));
}

#[tokio::test]
async fn oversized_feed_response_is_rejected_before_parsing() {
    for size in [ResponseSize::OversizedHeader, ResponseSize::OversizedStream] {
        let (index, handle) = fake_feed("Swashbuckle.AspNetCore".into(), size);
        let client = NugetClient::with_source(&index).unwrap();
        let error = client.search("Swashbuckle.AspNetCore", false, 0, 10).await.unwrap_err();
        assert!(error.to_string().contains("exceeds 4 MiB"), "{error}");
        assert_eq!(handle.join().unwrap().len(), 2);
    }
}
