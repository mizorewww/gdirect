//! A local CONNECT proxy that sends Google hosts to verified endpoints, so a
//! tool that only knows the system network (rclone, for `mount`) works
//! without a VPN. TLS runs end to end between the tool and Google with the
//! real hostname; the proxy only picks the IP. Anything that is not a Google
//! host on port 443 is refused.
use anyhow::Result;
use std::{
    io::{self, BufRead, BufReader, Write},
    net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Longest request head accepted from the local client.
const MAX_HEAD: usize = 8192;

fn google_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    ["googleapis.com", "google.com", "googleusercontent.com"]
        .iter()
        .any(|domain| {
            host == *domain
                || host
                    .strip_suffix(domain)
                    .is_some_and(|rest| rest.ends_with('.'))
        })
}
/// Endpoints in order; the one that last worked is tried first.
struct Upstream {
    endpoints: Vec<SocketAddr>,
    current: AtomicUsize,
}
impl Upstream {
    fn connect(&self) -> io::Result<TcpStream> {
        let start = self.current.load(Ordering::Relaxed);
        let mut last = io::Error::new(io::ErrorKind::NotFound, "no endpoint");
        for i in 0..self.endpoints.len() {
            let index = (start + i) % self.endpoints.len();
            match TcpStream::connect_timeout(&self.endpoints[index], CONNECT_TIMEOUT) {
                Ok(stream) => {
                    self.current.store(index, Ordering::Relaxed);
                    return Ok(stream);
                }
                Err(error) => last = error,
            }
        }
        Err(last)
    }
}
/// Starts the proxy on a free loopback port and returns its address. It runs
/// until the process exits.
pub fn spawn(endpoints: Vec<IpAddr>) -> Result<SocketAddr> {
    spawn_to(
        endpoints
            .into_iter()
            .map(|ip| SocketAddr::new(ip, 443))
            .collect(),
    )
}
fn spawn_to(endpoints: Vec<SocketAddr>) -> Result<SocketAddr> {
    anyhow::ensure!(!endpoints.is_empty(), "no endpoint for the proxy");
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    let address = listener.local_addr()?;
    let upstream = Arc::new(Upstream {
        endpoints,
        current: AtomicUsize::new(0),
    });
    thread::spawn(move || {
        for client in listener.incoming().flatten() {
            let upstream = upstream.clone();
            thread::spawn(move || {
                let _ = tunnel(client, &upstream);
            });
        }
    });
    Ok(address)
}
fn reply(client: &mut TcpStream, status: &str) -> io::Result<()> {
    client.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\n\r\n").as_bytes())
}
fn tunnel(mut client: TcpStream, upstream: &Upstream) -> io::Result<()> {
    client.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(client.try_clone()?);
    let mut size = 0;
    let mut first = String::new();
    loop {
        let mut line = String::new();
        size += reader.read_line(&mut line)?;
        if line.is_empty() || size > MAX_HEAD {
            return Ok(());
        }
        if first.is_empty() {
            first = line;
        } else if line == "\r\n" || line == "\n" {
            break;
        }
    }
    let mut words = first.split_whitespace();
    let target = match (words.next(), words.next()) {
        (Some("CONNECT"), Some(target)) => target,
        _ => return reply(&mut client, "405 Method Not Allowed"),
    };
    let allowed = target
        .rsplit_once(':')
        .is_some_and(|(host, port)| port == "443" && google_host(host));
    if !allowed {
        return reply(&mut client, "403 Forbidden");
    }
    let Ok(mut server) = upstream.connect() else {
        return reply(&mut client, "502 Bad Gateway");
    };
    client.set_read_timeout(None)?;
    client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
    // Bytes the client sent right after the head are already buffered.
    server.write_all(reader.buffer())?;
    let mut to_server = (reader.into_inner(), server.try_clone()?);
    let sender = thread::spawn(move || {
        let _ = io::copy(&mut to_server.0, &mut to_server.1);
        let _ = to_server.1.shutdown(Shutdown::Write);
    });
    let _ = io::copy(&mut server, &mut client);
    let _ = client.shutdown(Shutdown::Write);
    let _ = sender.join();
    Ok(())
}
/// Points a child process's HTTPS traffic at the proxy.
pub fn apply(command: &mut std::process::Command, proxy: SocketAddr) {
    let url = format!("http://{proxy}");
    for key in ["HTTPS_PROXY", "https_proxy"] {
        command.env(key, &url);
    }
    for key in ["ALL_PROXY", "all_proxy", "NO_PROXY", "no_proxy"] {
        command.env_remove(key);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    fn request(proxy: SocketAddr, head: &str) -> (String, TcpStream) {
        let mut stream = TcpStream::connect(proxy).unwrap();
        stream.write_all(head.as_bytes()).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut status = String::new();
        reader.read_line(&mut status).unwrap();
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap() > 0 && line != "\r\n" {
            line.clear();
        }
        (status, stream)
    }
    #[test]
    fn only_google_hosts_pass() {
        for host in [
            "www.googleapis.com",
            "oauth2.googleapis.com",
            "drive.usercontent.google.com",
            "doc-0s-1c-docs.googleusercontent.com",
            "google.com",
            "WWW.GOOGLEAPIS.COM",
        ] {
            assert!(google_host(host), "{host}");
        }
        for host in [
            "example.com",
            "notgoogle.com",
            "google.com.evil.example",
            "googleapis.com.cn",
            "",
        ] {
            assert!(!google_host(host), "{host}");
        }
    }
    #[test]
    fn tunnels_to_the_next_working_endpoint() {
        let echo = TcpListener::bind("127.0.0.1:0").unwrap();
        let target = echo.local_addr().unwrap();
        thread::spawn(move || {
            for mut stream in echo.incoming().flatten() {
                thread::spawn(move || {
                    let mut buf = [0; 4];
                    stream.read_exact(&mut buf).unwrap();
                    stream.write_all(&buf).unwrap();
                });
            }
        });
        // A closed port first: the proxy fails over and remembers the live one.
        let dead = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let proxy = spawn_to(vec![dead, target]).unwrap();
        for _ in 0..2 {
            let (status, mut stream) = request(
                proxy,
                "CONNECT www.googleapis.com:443 HTTP/1.1\r\nHost: www.googleapis.com:443\r\n\r\n",
            );
            assert!(status.starts_with("HTTP/1.1 200"), "{status}");
            stream.write_all(b"ping").unwrap();
            let mut buf = [0; 4];
            stream.read_exact(&mut buf).unwrap();
            assert_eq!(&buf, b"ping");
        }
        let (status, _) = request(proxy, "CONNECT example.com:443 HTTP/1.1\r\n\r\n");
        assert!(status.starts_with("HTTP/1.1 403"), "{status}");
        let (status, _) = request(proxy, "CONNECT www.googleapis.com:80 HTTP/1.1\r\n\r\n");
        assert!(status.starts_with("HTTP/1.1 403"), "{status}");
        let (status, _) = request(proxy, "GET http://www.googleapis.com/ HTTP/1.1\r\n\r\n");
        assert!(status.starts_with("HTTP/1.1 405"), "{status}");
    }
}
