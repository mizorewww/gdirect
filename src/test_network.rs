//! Exercise real HTTP routing in child processes so proxy variables cannot affect other tests.
use anyhow::Result;
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

fn serve(
    listener: TcpListener,
    route: &str,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<Option<String>> {
    let body = serde_json::json!({"route": route}).to_string();
    listener.set_nonblocking(true).unwrap();
    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(50);
        while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut first = String::new();
                    reader.read_line(&mut first).unwrap();
                    let mut line = String::new();
                    let mut length = 0;
                    loop {
                        line.clear();
                        let bytes = reader.read_line(&mut line).unwrap();
                        if bytes == 0 || line == "\r\n" {
                            break;
                        }
                        if let Some((name, value)) = line.split_once(':')
                            && name.eq_ignore_ascii_case("content-length")
                        {
                            length = value.trim().parse::<usize>().unwrap();
                        }
                    }
                    assert!(length < 8192, "unexpected fixture body size");
                    reader.read_exact(&mut vec![0; length]).unwrap();
                    let status = if first.starts_with("CONNECT ") {
                        "502 Bad Gateway"
                    } else {
                        "200 OK"
                    };
                    write!(
                        stream,
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .unwrap();
                    return Some(first);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("fixture accept failed: {error}"),
            }
        }
        None
    })
}

pub fn assert_route(test: &str, proxied: bool, request: impl FnOnce(&str) -> Result<Value>) {
    if std::env::var("GDIRECT_TEST_NETWORK").as_deref() == Ok(test) {
        let url = std::env::var("GDIRECT_TEST_URL").unwrap();
        if url.starts_with("https:") {
            // The proxy rejects CONNECT before TLS, without changing certificate verification.
            assert!(request(&url).is_err());
        } else {
            let data = request(&url).unwrap();
            assert_eq!(data["route"], if proxied { "proxy" } else { "direct" });
        }
        return;
    }

    run_fixture(test, proxied, "http");
    if proxied {
        run_fixture(test, proxied, "https");
    }
}

fn run_fixture(test: &str, proxied: bool, scheme: &str) {
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy_url = format!("http://{}", proxy.local_addr().unwrap());
    let host = if proxied {
        "proxy-route.invalid"
    } else {
        "www.googleapis.com"
    };
    let url = format!(
        "{scheme}://{host}:{}/fixture",
        origin.local_addr().unwrap().port()
    );
    let stop = Arc::new(AtomicBool::new(false));
    let proxy_thread = serve(proxy, "proxy", stop.clone());
    let origin_thread = serve(origin, "direct", stop.clone());
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", test, "--nocapture"])
        .env("GDIRECT_TEST_NETWORK", test)
        .env("GDIRECT_TEST_URL", url)
        .env_remove("REQUEST_METHOD");
    for name in [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        command.env(name, &proxy_url);
    }
    command.env("NO_PROXY", "").env("no_proxy", "");
    let output = command.output();
    stop.store(true, Ordering::Relaxed);
    let proxy_used = proxy_thread.join().unwrap();
    let origin_used = origin_thread.join().unwrap();
    let output = output.unwrap();
    assert!(
        output.status.success(),
        "routing subprocess failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        (proxy_used.is_some(), origin_used.is_some()),
        (proxied, !proxied)
    );
    if scheme == "https" {
        assert!(
            proxy_used
                .unwrap()
                .starts_with("CONNECT proxy-route.invalid:")
        );
    }
}
