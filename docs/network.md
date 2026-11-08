# How gdirect reaches Google

[中文](network.zh-CN.md) · [README](../README.md)

| Traffic | Route |
| --- | --- |
| Drive API, downloads, uploads and their token refresh (`browse`, `ls`, `download`, `upload`) | A scanned Google IP. The hostname and certificate check stay the same. If connections carrying the real SNI are reset, gdirect automatically retries under a forged identity (`g.cn` as TLS name, the real host in the Host header) — same idea as Sheas-Cealer — and remembers what worked per IP |
| Sign-in and account verification (`accounts.google.com`, `oauth2.googleapis.com`, `www.googleapis.com`), and token refresh when no scanned IP answers for it | Your normal system network (DNS, proxy) |
| `mount` (rclone) | A scanned Google IP, through a local proxy that gdirect runs for rclone (TLS stays end to end with the real hostname). Your normal system network only if no IP can be found |

The first transfer command sweeps Google's own front ends and keeps the ones that work for 24 hours. They come from the address space Google publishes as its own ([goog.json minus cloud.json](https://knowledge.workspace.google.com/admin/security/obtain-google-ip-address-ranges), embedded and refreshed weekly). Most Google IPs are blocked in mainland China, but some front ends are not: the sweep tries a TCP connection first, fully verifies only what answers (TLS, the Drive API and a test download), and stops once it has 8. When the machine has a global IPv6 address, Google's IPv6 front ends are swept too, mixed in with IPv4. Each candidate is probed with the real SNI first, then with the forged identity. If an endpoint stops answering, requests and downloads move to the next verified one. `login` does not scan. Certificates are verified against Google's own GTS roots (embedded in the binary), never against a system CA bundle — a compromised third-party CA cannot impersonate Google for gdirect.

Network requirements:

- Sign-in needs browser access to `accounts.google.com` and application access to `oauth2.googleapis.com` and `www.googleapis.com`. If Google is blocked, keep a proxy available for sign-in. During transfers, token refresh goes through the scanned IP too, so no proxy is needed then; only if that IP does not answer as Google's token endpoint does gdirect fall back to the system network.
- If you use a TUN-mode proxy or VPN (for example Clash/Mihomo TUN), route Google's IP ranges **DIRECT** and turn off fake-ip for them. A TUN proxy accepts every connection, so otherwise the scan measures the proxy instead of your network; `scan` warns when it sees this. `gdirect scan --list` shows the IPs in use.

### System proxy without TUN

- On macOS and Windows, sign-in and account verification use the manual HTTP/HTTPS system proxy (token refresh too, when it falls back to the system network). A proxy client in rule mode with TUN off is enough: proxy `accounts.google.com`, `oauth2.googleapis.com` and `www.googleapis.com`. No DIRECT rules are needed; scans, Drive API requests, uploads and downloads still connect to the scanned endpoints directly.
- If you turn TUN on with domain sniffing, a `www.googleapis.com` proxy rule also catches those transfers. Exclude the scanned IPs from sniffing, or send all of gdirect's traffic DIRECT with a process rule such as Clash's `PROCESS-NAME,gdirect,DIRECT`.
- On Linux, or with PAC, set `HTTPS_PROXY` for each command (your client's HTTP or mixed port). `HTTP_PROXY` / `HTTPS_PROXY` take precedence over system settings. `NO_PROXY` must not exclude the Google endpoints above.

```sh
HTTPS_PROXY=http://127.0.0.1:7890 gdirect login --client-secret ~/Downloads/client_secret.json
```
