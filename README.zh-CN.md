# gdirect

[English](README.md) · [更新日志](CHANGELOG.md)

gdirect 是一个在终端里浏览、下载、上传和挂载 Google 云盘的工具。它会找出当前网络能连通的 Google IP，让云盘流量直接连过去，下载支持多连接和断点续传。

## 工作原理

| 流量 | 走向 |
| --- | --- |
| Drive API、下载、上传（`browse`、`ls`、`download`、`upload`） | 扫描到的 Google IP。主机名和证书校验不变，可用 `--sni` / `--no-sni` 改写或去掉 TLS SNI |
| 登录、账号验证与刷新令牌（`accounts.google.com`、`oauth2.googleapis.com`、`www.googleapis.com`） | 系统网络（系统 DNS、代理） |
| `mount`（rclone） | 系统网络 |

第一次运行传输类命令时会自动扫描候选 IP，并在 24 小时内沿用最快的可用 IP。`login` 和 `mount` 不扫描。

网络要求：

- 登录时，浏览器需要访问 `accounts.google.com`，程序需要访问 `oauth2.googleapis.com` 和 `www.googleapis.com`；刷新令牌需要访问 `oauth2.googleapis.com`。如果 Google 被屏蔽，请保持代理可用，传输期间刷新令牌也会使用它。
- 如果开着 TUN 模式的代理或 VPN（例如 Clash/Mihomo TUN），要把扫描到的 Google IP 设为 **DIRECT 直连**，并对它们关闭 fake-ip。否则 gdirect 的流量会被重新接管，扫描结果为空。`gdirect scan --list` 可以查看这些 IP。

### 只开系统代理，不开 TUN

- 在 macOS 和 Windows 上，登录、账号验证与刷新令牌会使用系统中手动配置的 HTTP/HTTPS 代理。代理客户端保持规则模式、关闭 TUN 即可：让 `accounts.google.com`、`oauth2.googleapis.com` 和 `www.googleapis.com` 走代理。无需添加 DIRECT 规则；扫描、Drive API 请求、上传和下载仍直连扫描到的 IP。
- 如果开启 TUN 并启用了域名嗅探，`www.googleapis.com` 的代理规则也会命中这些传输，请对扫描到的 IP 关闭嗅探。
- 在 Linux 上或使用 PAC 时，请为每条命令设置 `HTTPS_PROXY`（客户端的 HTTP 或 mixed 端口）。`HTTP_PROXY` / `HTTPS_PROXY` 优先于系统设置，`mount` 也会把它们传给 rclone。`NO_PROXY` 不能排除上述 Google 端点。

```sh
HTTPS_PROXY=http://127.0.0.1:7890 gdirect login --client-secret ~/Downloads/client_secret.json
HTTPS_PROXY=http://127.0.0.1:7890 gdirect mount
```

## 安装

预编译包支持以下平台：

- macOS 15+（Apple Silicon / Intel）
- Linux x86_64，glibc 2.35+（Ubuntu 22.04+、Debian 12+ 及较新的 Fedora/Arch/openSUSE）
- Windows x86_64

Alpine/musl、旧版 glibc 和 Linux ARM64 需要[从源码编译](#从源码编译)。

macOS / Linux（需要 `curl`）：

```sh
curl -fsSL https://raw.githubusercontent.com/mizorewww/gdirect/main/scripts/install.sh | bash
```

Windows PowerShell：

```powershell
& ([scriptblock]::Create((Invoke-RestMethod 'https://raw.githubusercontent.com/mizorewww/gdirect/main/scripts/install.ps1')))
```

安装脚本会把 gdirect 放到 `~/.local/bin`（Windows 为 `%LOCALAPPDATA%\gdirect\bin`），并提示如何加入 PATH。Windows 装完后请重新打开终端。

要使用挂载功能，在命令末尾加 `--with-mount`（PowerShell 用 `-WithMount`），脚本会顺带安装依赖：

- 所有平台都会安装 rclone 1.75.2+。
- Linux 还会通过包管理器安装 FUSE3，需要 sudo。
- Windows 通过 winget 安装 rclone 和 WinFsp，安装 WinFsp 时可能弹出管理员授权。

Homebrew（macOS / Linux）：

```sh
brew tap mizorewww/gdirect https://github.com/mizorewww/gdirect
brew install mizorewww/gdirect/gdirect
brew install rclone   # 仅挂载需要；Linux 还要用发行版的包管理器安装 fuse3
```

升级可以执行 `brew update && brew upgrade gdirect`，也可以重新运行安装脚本。

也可以直接下载 [Release 安装包](https://github.com/mizorewww/gdirect/releases/latest)。解压后请把 `cacert.pem` 和可执行文件放在同一个目录，下载时要用它校验证书。

## 一次性配置 Google（约 10 分钟）

gdirect 不提供共用的 Google 客户端，需要你在自己的 Google Cloud 项目里创建一个，只需做一次。按照 [Google 配置指南](docs/configuration/README.zh-CN.md)下载 **Desktop app（桌面应用）** 类型的 OAuth 客户端 JSON，然后执行：

```sh
gdirect login --client-secret ~/Downloads/client_secret.json
```

浏览器会自动打开，授权完成后终端显示 `Logged in`。之后就不再需要这个 JSON 文件了。

## 日常使用

```sh
gdirect                                   # 等同于 gdirect browse
gdirect browse -o ~/Downloads             # 下载目录（默认 ./downloads）
gdirect ls root --json
gdirect download 文件ID或分享链接          # 保存到 ./downloads/<文件名>
gdirect download FILE_ID -o file.zip
gdirect download DOC_ID --export application/pdf -o doc.pdf
gdirect upload ./file.zip
gdirect upload ./file.zip --parent FOLDER_ID --name backup.zip
```

`browse` 仿照 [yazi](https://github.com/sxyazi/yazi)，并排显示上级文件夹、当前文件夹和预览。按 `?` 查看全部按键。

| `browse` 中的按键 | 操作 |
| --- | --- |
| `j/k`、`↑/↓` | 选择 |
| `g/G`、Home/End | 跳到第一项 / 最后一项 |
| `Ctrl-d/Ctrl-u`、PgDn/PgUp | 向下或向上翻半页 / 一页 |
| Enter、`l/→` | 进入文件夹 |
| `h/←`、Backspace | 返回上级 |
| `/` | 按名称筛选当前文件夹。Enter 保留筛选，Esc 清除 |
| `Ctrl-r`、F5 | 刷新 |
| 空格 | 标记或取消标记；切换文件夹后标记仍保留 |
| `Ctrl-a` | 标记当前显示的所有项目（再按一次取消） |
| Esc | 依次清除筛选、标记、剪贴板 |
| `a` | 新建空文件；名称以 `/` 结尾则新建文件夹 |
| `r` | 重命名 |
| `y` / `x` | 复制 / 剪切已标记的项目（没有标记时为当前选中的项目） |
| `p` | 粘贴到当前文件夹。剪切的项目会被移动，复制的文件会生成副本（Drive 不支持复制文件夹） |
| `D`、Delete | 把已标记的项目（没有标记时为当前选中的项目）移到 Drive 回收站（会先确认） |
| `u` | 上传本地文件到当前文件夹 |
| `d` | 把已标记的项目（没有标记时为当前选中的项目）加入下载队列。文件夹会连同所有子文件夹一起下载到 `<下载目录>/<文件夹名>/`；Google 文档、表格、幻灯片导出为 PDF |
| `+/-` | 调整下一个下载的连接数（1–16），并保存为默认值 |
| `c` | 取消当前下载并保留已下载部分 |
| `R` | 重试失败或已取消的下载 |
| `X` | 移除尚未开始的下载 |
| `q` | 退出 |

- **队列：** 下载逐个进行。文件列表会显示每个文件的状态（◦ 排队、↓ 下载中、✓ 已保存、✗ 失败），下载面板显示整体进度。选中失败的文件可以看到完整错误。同一会话中同名的文件会保存为 `name (2).ext`。
- **续传：** 下载或上传中断后，重新执行同一条命令即可接着传。
- **下载：** 已经下载完成的文件会按校验和识别出来，提示已下载。每次下载完成后都会与 Drive 记录的 MD5 比对。
- **上传：** 总是新建文件，不会覆盖云端的同名文件。
- **连接数：** `-j N` 只对当前这条命令生效。保存的默认值（初始为 8）只能在 `browse` 里用 `+/-` 修改。

`download` 和 `browse` 支持以下高级传输选项（`ls`、`upload` 只支持 `--ip`）：

| 选项 | 说明 |
| --- | --- |
| `--ip IP` | 指定 Google IP，不使用扫描结果 |
| `--sni HOST` / `--no-sni` | 发送其他 TLS SNI，或不发送 SNI。证书仍按真实主机名校验 |
| `--ca-cert FILE` | 用指定的 CA 证书包代替自带的 `cacert.pem` |
| `--export MIME` | （`download`）导出 Google 文档，例如 `application/pdf` |

`gdirect scan` 会重新扫描，`gdirect scan --list` 查看扫描结果，其中包括每个 IP 最近一次的错误。要测试自己找到的候选 IP，可以执行 `gdirect scan --ips FILE`：文件中每行写一个公网 IP，`#` 后面是注释。

写脚本时可以用 `GDIRECT_ACCESS_TOKEN` 直接提供访问令牌，不必登录（`mount` 仍然需要先 `gdirect login`）。

## 挂载

挂载需要先通过 `--with-mount` 或上面的 Homebrew 步骤装好依赖。

```sh
gdirect mount                        # 挂到 ~/GoogleDrive
gdirect mount ~/MyDrive --read-only
gdirect unmount
```

- macOS 使用系统自带的 NFS，不需要 FUSE 驱动；Linux 使用 FUSE3。
- Windows 在前台运行，按 Ctrl-C 停止。
- macOS/Linux 的挂载目录必须为空；Windows 的挂载目录必须是还不存在的路径。
- 写入的文件先进本地缓存，再在后台上传。缓存会尽量控制在 5 GiB 以内，但正在使用或尚未上传的文件可能让它超出。`mount` 会打印日志路径，日志里出现 `to upload 0, uploading 0` 就说明上传已经完成。
- `unmount` 会保留还没上传的文件，下次 `mount` 时继续上传。
- `-j` 控制 rclone 同时进行的传输数。

## 本地数据与退出登录

凭据、扫描到的 IP 和设置都保存在一个私有的 SQLite 文件中：

- macOS：`~/Library/Application Support/gdirect/state.sqlite3`
- Linux：`~/.local/share/gdirect/state.sqlite3`
- Windows：`%LOCALAPPDATA%\gdirect\state.sqlite3`

可以用 `--db PATH` 指定其他文件。`gdirect logout` 会删除已保存的凭据，保留扫描结果；退出登录前请先卸载。切换 Google 账号用 `gdirect login --reauth`。

登录失败、7 天过期等问题请看[常见问题](docs/configuration/README.zh-CN.md#常见问题)。

## 从源码编译

需要 Rust 1.91+、C++ 编译器、make、curl 和 OpenSSL 3。

```sh
git clone https://github.com/mizorewww/gdirect.git
cd gdirect
# macOS：brew install openssl@3
# Debian / Ubuntu：sudo apt-get install build-essential pkg-config libssl-dev ca-certificates curl xz-utils
./scripts/build-libaria2.sh
cargo install --path . --locked
```

Windows 需要使用 MinGW GNU 工具链编译，步骤如下：

1. 在 MSYS2 的 MINGW64 终端中安装依赖：`make patch tar xz mingw-w64-x86_64-gcc mingw-w64-x86_64-openssl mingw-w64-x86_64-pkgconf`。
2. 执行 `rustup target add x86_64-pc-windows-gnu`。
3. 设置环境变量 `OPENSSL_PREFIX=$(cygpath -m /mingw64)`。
4. 执行上面的两条构建命令，并给 `cargo install` 加上 `--target x86_64-pc-windows-gnu`。
