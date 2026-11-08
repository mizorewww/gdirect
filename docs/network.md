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
- A TUN-mode proxy or VPN (sing-box, Clash/Mihomo, WireGuard…) needs no setup: see [TUN proxies and VPNs](#tun-proxies-and-vpns). `gdirect scan --list` shows the IPs in use and how connections leave the machine.

### TUN proxies and VPNs

A TUN proxy takes the default route, so a connection to a scanned Google IP would enter the tunnel, have its SNI sniffed, and go wherever the proxy's rules send Google — usually through a proxy node. The scan would then measure the proxy, and "direct" transfers would run over (often metered) proxy traffic. Process rules such as `PROCESS-NAME,gdirect,DIRECT` do not help with every client (sing-box on macOS reports no process for TUN connections).

So, by default, gdirect binds every connection to a scanned endpoint to the physical network adapter the TUN would capture, the way sing-box's own `auto_detect_interface` escapes its TUN:

| System | How the adapter is found | How connections are bound |
| --- | --- | --- |
| macOS | the routing table's default routes (with a TUN up, the adapter's scoped default route is still there), skipping utun, ipsec, ppp, bridge, awdl and similar interfaces | `IP_BOUND_IF` / `IPV6_BOUND_IF` |
| Linux | the main table's default route (`/proc/net/route`), skipping tun, wg, tailscale, Meta and similar interfaces; policy-routing TUNs keep theirs in other tables | `SO_BINDTODEVICE` / `SO_BINDTOIFINDEX`, which need `CAP_NET_RAW`: `sudo setcap cap_net_raw+ep "$(command -v gdirect)"`. Without it gdirect stays on the system route and `scan` says so |
| Windows | the IPv4 default route with the lowest metric, skipping virtual and tunnel adapters (Wintun, sing-box, Clash, WireGuard, TAP…) | `IP_UNICAST_IF` / `IPV6_UNICAST_IF`; Drive API requests bind the adapter's address |

This covers scans, Drive API requests, uploads, downloads (aria2), token refresh over a scanned IP, and the local proxy `mount` runs for rclone. Sign-in, account checks and the system-network fallbacks are never bound: they follow your proxy. Without a TUN the adapter already carries the default route, so binding changes nothing.

To let your proxy rules decide instead (for example to send Drive through a VPN on purpose), turn it off: in the desktop client under Settings › Network, or with `gdirect scan --bypass-tun off` (saved; `--bypass-tun on` turns it back on). `GDIRECT_BYPASS_TUN=0` or `=1` overrides the saved choice for one command.

gdirect records the network path the saved IPs were verified over (`direct:en0`, or `tun:utun4` when a TUN carries the traffic). When it changes — the setting is switched, you move to another adapter, or a TUN comes or goes while bypassing is off — the saved IPs are re-verified by a new scan before use. An unchanged path never triggers one.

### System proxy without TUN

- On macOS and Windows, sign-in and account verification use the manual HTTP/HTTPS system proxy (token refresh too, when it falls back to the system network). A proxy client in rule mode with TUN off is enough: proxy `accounts.google.com`, `oauth2.googleapis.com` and `www.googleapis.com`. No DIRECT rules are needed; scans, Drive API requests, uploads and downloads still connect to the scanned endpoints directly.
- If you turn TUN on with domain sniffing, gdirect's transfers still bypass it (see [TUN proxies and VPNs](#tun-proxies-and-vpns)) while sign-in keeps using the proxy.
- On Linux, or with PAC, set `HTTPS_PROXY` for each command (your client's HTTP or mixed port). `HTTP_PROXY` / `HTTPS_PROXY` take precedence over system settings. `NO_PROXY` must not exclude the Google endpoints above.

```sh
HTTPS_PROXY=http://127.0.0.1:7890 gdirect login --client-secret ~/Downloads/client_secret.json
```
