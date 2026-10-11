use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn mock(responses: Vec<Value>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        let mut requests = vec![];
        for body in responses {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut data = vec![];
            loop {
                let mut chunk = [0u8; 4096];
                let count = stream.read(&mut chunk).await.unwrap();
                if count == 0 {
                    break;
                }
                data.extend_from_slice(&chunk[..count]);
                if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&data[..end]);
                    let length = headers
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|s| s.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if data.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8(data).unwrap());
            let body = body.to_string();
            let reply = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            stream.write_all(reply.as_bytes()).await.unwrap();
        }
        requests
    });
    (root, handle)
}
fn config() -> Config {
    Config {
        owner_id: "router-test".into(),
        gpu_type_ids: vec!["NVIDIA GeForce RTX 5090".into()],
        ..Default::default()
    }
}
fn params() -> CreateInstanceParams<'static> {
    CreateInstanceParams {
        client: "me",
        image: "strata:test",
        price: None,
        disk: Some(150),
        label: Some("praxis-llm-s1-12345678"),
        template_hash_id: None,
        onstart_cmd: None,
        runtype: Some("args"),
        env: json!({"PRAXIS_NODE_TOKEN":"private-node-key", "ROUTER_URL":"ws://router:8080/api/v1/node"}),
        extra: None,
        image_login: None,
        python_utf8: false,
        lang_utf8: false,
        use_jupyter_lab: false,
        jupyter_dir: None,
        force: false,
        cancel_unavail: false,
        user: None,
    }
}
#[test]
fn payload_has_persistent_storage_and_no_public_ports() {
    let r = Runpod::new("test-key".into(), config()).unwrap();
    let body = r.create_body("NVIDIA GeForce RTX 5090", &params()).unwrap();
    assert_eq!(body["env"]["PGPU_OWNER_ID"], "router-test");
    assert_eq!(body["env"]["STRATA_DATA"], "/workspace/strata");
    assert_eq!(body["env"]["PRAXIS_NODE_TOKEN"], "private-node-key");
    assert_eq!(body["volumeInGb"], 150);
    assert_eq!(body["ports"], json!([]));
    assert_eq!(body["interruptible"], false);
}
#[tokio::test]
async fn inventory_is_scoped_and_stop_uses_opaque_id() {
    let inventory = json!([
        {"id":"pod-owned","name":"praxis-llm-s1-12345678","desiredStatus":"RUNNING","env":{"PGPU_OWNER_ID":"router-test"}},
        {"id":"pod-foreign","desiredStatus":"RUNNING","env":{"PGPU_OWNER_ID":"another-router"}}
    ]);
    let (root, server) = mock(vec![inventory.clone(), inventory, Value::Null]).await;
    let r = Runpod::with_endpoints("test-key".into(), config(), &root, &root).unwrap();
    let instances = r.instances().await.unwrap();
    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].id, numeric_id("pod-owned"));
    r.set_status(instances[0].id, false).await.unwrap();
    let requests = server.await.unwrap();
    assert!(requests[2].starts_with("POST /pods/pod-owned/stop "));
    assert!(requests[0]
        .to_lowercase()
        .contains("authorization: bearer test-key"));
}
#[tokio::test]
async fn unverified_create_price_triggers_cleanup() {
    let quote = json!({"data":{"gpuTypes":[{"id":"NVIDIA GeForce RTX 5090","memoryInGb":32,"securePrice":0.70}]}});
    let (root, server) = mock(vec![
        quote,
        json!({"id":"new-pod","costPerHr":1.50}),
        Value::Null,
    ])
    .await;
    let r = Runpod::with_endpoints("test-key".into(), config(), &root, &root).unwrap();
    assert!(r
        .create(numeric_id("NVIDIA GeForce RTX 5090"), &params())
        .await
        .is_err());
    let requests = server.await.unwrap();
    assert!(requests[1].starts_with("POST /pods "));
    assert!(requests[2].starts_with("DELETE /pods/new-pod "));
}
#[tokio::test]
async fn resume_requotes_and_checks_returned_price() {
    let pod = json!({"id":"pod-owned","desiredStatus":"EXITED","volumeInGb":150,"costPerHr":0.7,
        "env":{"PGPU_OWNER_ID":"router-test","PGPU_GPU_TYPE_ID":"NVIDIA GeForce RTX 5090"}});
    let quote = json!({"data":{"gpuTypes":[{"id":"NVIDIA GeForce RTX 5090","memoryInGb":32,"securePrice":0.70}]}});
    let (root, server) = mock(vec![
        json!([pod.clone()]),
        pod.clone(),
        quote,
        Value::Null,
        pod,
    ])
    .await;
    let r = Runpod::with_endpoints("test-key".into(), config(), &root, &root).unwrap();
    r.set_status(numeric_id("pod-owned"), true).await.unwrap();
    let requests = server.await.unwrap();
    assert!(requests[3].starts_with("POST /pods/pod-owned/start "));
}
#[tokio::test]
async fn termination_never_touches_foreign_pods() {
    let (root, server) = mock(vec![
        json!([{"id":"foreign","env":{"PGPU_OWNER_ID":"another-router"}}]),
    ])
    .await;
    let r = Runpod::with_endpoints("test-key".into(), config(), &root, &root).unwrap();
    r.destroy(numeric_id("foreign")).await.unwrap();
    assert_eq!(server.await.unwrap().len(), 1);
}
#[tokio::test]
async fn malformed_inventory_fails_closed() {
    let (root, server) = mock(vec![json!({"pods":[]})]).await;
    let r = Runpod::with_endpoints("test-key".into(), config(), &root, &root).unwrap();
    assert!(r.instances().await.is_err());
    server.await.unwrap();
}
