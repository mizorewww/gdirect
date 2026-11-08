# gdirect 如何连到 Google

[English](network.md) · [README](../README.zh-CN.md)

| 流量 | 走向 |
| --- | --- |
| Drive API、下载、上传及其刷新令牌（`browse`、`ls`、`download`、`upload`） | 扫描到的 Google IP。主机名和证书校验不变。如果带真实 SNI 的连接被重置，会自动改用伪造身份（TLS 名称用 `g.cn`，Host 头仍是真实主机），并按 IP 记住哪种方式可用 |
| 登录与账号验证（`accounts.google.com`、`oauth2.googleapis.com`、`www.googleapis.com`），以及扫描到的 IP 无法刷新令牌时 | 系统网络（系统 DNS、代理） |
| `mount`（rclone） | 扫描到的 Google IP，经由 gdirect 为 rclone 启动的本地代理（TLS 仍按真实主机名端到端加密）。找不到可用 IP 时才走系统网络 |

第一次运行传输类命令时，会扫描 Google 自己的前端服务器，在 24 小时内沿用能用的那些。这些前端的地址来自 Google 公布为自有的 IP 段（[goog.json 减去 cloud.json](https://knowledge.workspace.google.com/admin/security/obtain-google-ip-address-ranges)，内置一份并每周更新）。在中国大陆，大部分 Google IP 被封，但仍有一部分前端可以直连：扫描先试 TCP 连接，只对能连上的做完整验证（TLS、Drive API 和测试下载），找到 8 个就停止。本机有公网 IPv6 地址时，也会扫描 Google 的 IPv6 前端，和 IPv4 穿插进行。每个候选 IP 先用真实 SNI 探测，失败再试伪造身份。某个 IP 失效时，请求和下载会自动换到下一个已验证的 IP。`login` 不扫描。证书只按内置的 Google GTS 根校验，不信任系统 CA 包——第三方 CA 被攻破也无法对 gdirect 冒充 Google。

网络要求：

- 登录时，浏览器需要访问 `accounts.google.com`，程序需要访问 `oauth2.googleapis.com` 和 `www.googleapis.com`。如果 Google 被屏蔽，登录时请保持代理可用。传输期间刷新令牌同样走扫描到的 IP，不需要代理；只有该 IP 不能作为 Google 令牌端点应答时，才回退到系统网络。
- 开着 TUN 模式的代理或 VPN（sing-box、Clash/Mihomo、WireGuard 等）时无需额外设置，见 [TUN 代理与 VPN](#tun-代理与-vpn)。`gdirect scan --list` 可以查看正在使用的 IP，以及连接从哪里出去。

### TUN 代理与 VPN

TUN 代理接管了默认路由，所以连向扫描到的 Google IP 的连接也会进入隧道，被嗅探出 SNI，再按代理规则发往 Google 对应的出口——通常是代理节点。这样扫描测到的是代理，所谓的直连传输其实在消耗（往往按量计费的）代理流量。`PROCESS-NAME,gdirect,DIRECT` 这类进程规则也不是处处有效（macOS 上的 sing-box 拿不到 TUN 连接的进程信息）。

因此 gdirect 默认把连向扫描到的 IP 的每条连接绑定到 TUN 原本要接管的物理网卡上，做法和 sing-box 自己用来绕开 TUN 的 `auto_detect_interface` 相同：

| 系统 | 如何找到网卡 | 如何绑定 |
| --- | --- | --- |
| macOS | 路由表中的默认路由（TUN 开着时，网卡的限定作用域默认路由仍在），跳过 utun、ipsec、ppp、bridge、awdl 等接口 | `IP_BOUND_IF` / `IPV6_BOUND_IF` |
| Linux | 主路由表的默认路由（`/proc/net/route`），跳过 tun、wg、tailscale、Meta 等接口；策略路由型 TUN 的路由在其他表里 | `SO_BINDTODEVICE` / `SO_BINDTOIFINDEX`，需要 `CAP_NET_RAW`：`sudo setcap cap_net_raw+ep "$(command -v gdirect)"`。没有该权限时 gdirect 仍走系统路由，`scan` 会提示 |
| Windows | 跃点数最小的 IPv4 默认路由，跳过虚拟和隧道网卡（Wintun、sing-box、Clash、WireGuard、TAP 等） | `IP_UNICAST_IF` / `IPV6_UNICAST_IF`；Drive API 请求绑定网卡地址 |

绑定覆盖扫描、Drive API 请求、上传、下载（aria2）、经扫描 IP 的令牌刷新，以及 `mount` 为 rclone 启动的本地代理。登录、账号验证和回退到系统网络的请求从不绑定，仍走你的代理。没有 TUN 时默认路由本来就是这块网卡，绑定不改变任何事。

如果想让代理规则来决定（例如有意让 Drive 走 VPN），可以关闭它：桌面客户端在“设置 › 网络”中，命令行用 `gdirect scan --bypass-tun off`（会保存；`--bypass-tun on` 重新打开）。`GDIRECT_BYPASS_TUN=0` 或 `=1` 只对单条命令覆盖保存的选择。

gdirect 会记录已保存的 IP 是经由哪条网络路径验证的（`direct:en0`，或 TUN 承载流量时的 `tun:utun4`）。路径变化时——切换了这个设置、换了网卡，或者在不绕过时 TUN 开启或关闭——已保存的 IP 会先重新扫描验证再使用。路径不变时不会触发重新扫描。

### 只开系统代理，不开 TUN

- 在 macOS 和 Windows 上，登录与账号验证会使用系统中手动配置的 HTTP/HTTPS 代理（回退到系统网络时的刷新令牌也一样）。代理客户端保持规则模式、关闭 TUN 即可：让 `accounts.google.com`、`oauth2.googleapis.com` 和 `www.googleapis.com` 走代理。无需添加 DIRECT 规则；扫描、Drive API 请求、上传和下载仍直连扫描到的节点。
- 如果开启 TUN 并启用了域名嗅探，gdirect 的传输仍会绕过它（见 [TUN 代理与 VPN](#tun-代理与-vpn)），登录照常走代理。
- 在 Linux 上或使用 PAC 时，请为每条命令设置 `HTTPS_PROXY`（客户端的 HTTP 或 mixed 端口）。`HTTP_PROXY` / `HTTPS_PROXY` 优先于系统设置。`NO_PROXY` 不能排除上述 Google 端点。

```sh
HTTPS_PROXY=http://127.0.0.1:7890 gdirect login --client-secret ~/Downloads/client_secret.json
```
