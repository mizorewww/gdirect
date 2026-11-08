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
- 如果开着 TUN 模式的代理或 VPN（例如 Clash/Mihomo TUN），要把 Google 的 IP 段设为 **DIRECT 直连**，并对它们关闭 fake-ip。TUN 代理会接受所有连接，否则扫描测到的是代理而不是你的网络；`scan` 发现这种情况时会提示。`gdirect scan --list` 可以查看正在使用的 IP。

### 只开系统代理，不开 TUN

- 在 macOS 和 Windows 上，登录与账号验证会使用系统中手动配置的 HTTP/HTTPS 代理（回退到系统网络时的刷新令牌也一样）。代理客户端保持规则模式、关闭 TUN 即可：让 `accounts.google.com`、`oauth2.googleapis.com` 和 `www.googleapis.com` 走代理。无需添加 DIRECT 规则；扫描、Drive API 请求、上传和下载仍直连扫描到的节点。
- 如果开启 TUN 并启用了域名嗅探，`www.googleapis.com` 的代理规则也会命中这些传输，请对扫描到的 IP 关闭嗅探，或用进程规则让 gdirect 的全部流量直连，例如 Clash 的 `PROCESS-NAME,gdirect,DIRECT`。
- 在 Linux 上或使用 PAC 时，请为每条命令设置 `HTTPS_PROXY`（客户端的 HTTP 或 mixed 端口）。`HTTP_PROXY` / `HTTPS_PROXY` 优先于系统设置。`NO_PROXY` 不能排除上述 Google 端点。

```sh
HTTPS_PROXY=http://127.0.0.1:7890 gdirect login --client-secret ~/Downloads/client_secret.json
```
