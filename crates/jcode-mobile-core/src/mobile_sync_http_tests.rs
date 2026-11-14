use super::*;
use std::io::{Read, Write};
use std::time::{Duration, Instant};

fn server(expected: usize) -> (Account, std::thread::JoinHandle<Vec<Vec<String>>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let account = Account {
        given_name: None,
        name: None,
        api_base: format!("http://{}/v1", listener.local_addr().unwrap()),
        api_key: "fixture-account".into(),
        email: None,
    };
    let task = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut connections = Vec::new();
        let mut count = 0;
        while count < expected {
            assert!(
                Instant::now() < deadline,
                "fixture did not receive all requests"
            );
            let (mut socket, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                }
                Err(error) => panic!("fixture accept: {error}"),
            };
            socket.set_nodelay(true).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            std::thread::sleep(Duration::from_millis(30));
            let mut requests = Vec::new();
            while count < expected {
                let mut bytes = Vec::new();
                loop {
                    let mut byte = [0];
                    match socket.read(&mut byte) {
                        Ok(0) => break,
                        Ok(_) => bytes.push(byte[0]),
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                || error.kind() == std::io::ErrorKind::TimedOut =>
                        {
                            break;
                        }
                        Err(error) => panic!("fixture read: {error}"),
                    }
                    if bytes.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                if bytes.is_empty() {
                    break;
                }
                assert!(bytes.ends_with(b"\r\n\r\n"));
                let headers = String::from_utf8(bytes).unwrap();
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                let mut body = vec![0; length];
                socket.read_exact(&mut body).unwrap();
                requests.push(headers);
                let body = br#"{"sessions":[],"commands":[]}"#;
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len()).unwrap();
                socket.flush().unwrap();
                std::thread::sleep(Duration::from_millis(8));
                socket.write_all(body).unwrap();
                socket.flush().unwrap();
                count += 1;
            }
            connections.push(requests);
        }
        connections
    });
    (account, task)
}

#[test]
fn requests_reuse_transport_without_retaining_account_headers_or_results() {
    let (mut account, task) = server(4);
    assert!(list(&account).unwrap().is_empty());
    account.api_key = "second-fixture-account".into();
    assert!(commands(&account).unwrap().is_empty());
    assert!(list(&account).unwrap().is_empty());
    account.api_key = "fixture-account".into();
    assert!(commands(&account).unwrap().is_empty());
    let connections = task.join().unwrap();
    assert_eq!(connections.len(), 1);
    let requests = &connections[0];
    assert_eq!(requests.len(), 4);
    for (index, key) in [
        "fixture-account",
        "second-fixture-account",
        "second-fixture-account",
        "fixture-account",
    ]
    .iter()
    .enumerate()
    {
        assert!(
            requests[index]
                .to_ascii_lowercase()
                .contains(&format!("authorization: bearer {key}\r\n"))
        );
    }
    assert!(requests[0].starts_with("GET /v1/mobile/sessions?limit=500 "));
    assert!(requests[1].starts_with("GET /v1/mobile/commands "));
}

#[test]
fn pooled_connections_are_origin_scoped() {
    let (first, first_task) = server(2);
    let (mut second, second_task) = server(2);
    second.api_key = "other-origin-account".into();
    for _ in 0..2 {
        assert!(list(&first).unwrap().is_empty());
        assert!(list(&second).unwrap().is_empty());
    }
    let first = first_task.join().unwrap();
    let second = second_task.join().unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    assert!(
        first[0]
            .iter()
            .all(|request| !request.contains("other-origin-account"))
    );
    assert!(
        second[0]
            .iter()
            .all(|request| request.contains("other-origin-account"))
    );
}

#[test]
fn invalid_command_ids_never_reach_transport() {
    let account = Account {
        given_name: None,
        name: None,
        api_base: "http://127.0.0.1:1".into(),
        api_key: "fixture-only".into(),
        email: None,
    };
    assert_eq!(
        command(&account, "../invalid").unwrap_err(),
        "Invalid command id"
    );
}

#[test]
#[ignore = "controlled loopback connection setup benchmark, not device or WAN latency"]
fn benchmark_mobile_sync_http_pool() {
    for _ in 0..3 {
        let (account, task) = server(8);
        let started = Instant::now();
        for _ in 0..8 {
            assert!(list(&account).unwrap().is_empty());
        }
        let elapsed = started.elapsed();
        let connections = task.join().unwrap();
        println!(
            "mobile_sync_http requests=8 connections={} duration_ms={:.3}",
            connections.len(),
            elapsed.as_secs_f64() * 1000.0
        );
    }
}
