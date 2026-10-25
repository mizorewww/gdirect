# GDirect macOS 图形界面

原生 SwiftUI 界面通过内嵌、固定版本的 gdirect CLI 操作 Google Drive。系统要求为 macOS 15 或更新版本，支持本机 arm64 和 x86_64 架构；构建要求 Swift 6.4 或更新版本、Xcode Command Line Tools、Python 3，以及系统提供的 curl、tar、codesign 等工具。无需手动创建 Xcode 工程。

## 构建与启动

在仓库根目录执行：

```sh
./macos/build-app.sh
open macos/dist/GDirect.app
```

脚本运行 Swift 测试、编译 release GUI，按本机架构下载 `Resources/CLIRelease.json` 固定的官方 CLI 发布包，核验 SHA256、CLI 版本、架构、CA、Info.plist 与签名，最终生成 `macos/dist/GDirect.app`。当前固定版本为 gdirect 0.2.0。构建需要联网下载发布包；不自动跟随上游 main，也不自动升级现有应用。

本地调试可显式指定已有的 CLI，路径必须为绝对路径：

```sh
./macos/build-app.sh --cli /absolute/path/to/gdirect
```

本地 CLI 同样必须匹配固定版本和本机架构；此入口不校验发布包 SHA256。脚本优先使用 CLI 同目录的 `cacert.pem`，其次查找系统 CA；也可通过 `GDIRECT_CA_FILE=/absolute/path/to/cacert.pem` 指定。应用内实际证书位于 `Contents/Resources/cacert.pem`，`Contents/MacOS/cacert.pem` 为对应链接。

单独执行 `swift build --package-path macos` 仅生成 GUI 可执行文件，不组装包含 CLI 和 CA 的可用 APP。

## Google 账号配置

本项目不提供共用 OAuth 客户端。按照仓库的[Google 配置指南](../docs/configuration/README.zh-CN.md)在自己的 Google Cloud 项目中启用 Drive API、配置 Drive 权限范围，并创建 **Desktop app（桌面应用）** 类型的 OAuth 客户端。若 OAuth 应用仍处于测试状态，应将所用账号加入测试用户。下载客户端 JSON 后：

1. 启动 APP，点击顶部 **Account…**。
2. 选择 **Desktop OAuth client** 登录方式，选择客户端 JSON，点击 **Sign In**；留空时使用 CLI 已保存的客户端。
3. 在本机打开的浏览器中核对应用名称与权限并完成授权。回调使用本机临时端口，不能转到其他设备完成。
4. 返回应用，确认账号连接状态并刷新文件列表。

登录方式为互斥选择，只有当前所选方式的文件会传给 CLI。**Existing credentials** 可导入已有的 `authorized_user` 凭据 JSON；该文件应包含 Drive 权限。**Reauthorize…** 用于重新授权或切换账号，**Log Out** 删除 CLI 保存的凭据。存在排队、运行或正在结束的传输时，账号操作会显示阻止提示。账号操作执行期间新增传输会显示失败提示，完成后可重试。成功登录或重新授权后，文件浏览回到根目录并重置导航历史；注销后清空文件列表。重新授权后，应先卸载再重新挂载，使挂载使用新凭据。OAuth 登录所需的 Google 账号与令牌端点仍须能够从当前网络访问。

客户端 JSON、凭据 JSON 和 CLI 的私有 SQLite 数据库均应妥善保管，不应提交到仓库或分享。凭据由 CLI 管理；GUI 不直接读写数据库，仅将非敏感偏好保存在 UserDefaults，诊断信息只在内存中限长保留。

## 文件与任务

在 **Files** 中双击文件夹进入，使用返回、前进和刷新按钮导航。选中非文件夹项目后，通过工具栏或右键菜单点击 **Download**，界面先显示确认对话框，确认后保存到设置中的默认目录。Google Docs、Sheets、Slides、Drawings 默认导出 PDF；只有这四类支持 **Export…** 自定义 MIME 类型。快捷方式、Forms 等不受 CLI 支持的 Google 应用类型会显示说明对话框，不加入任务队列。**Public Link** 接受公开 Google Drive 链接，并要求选择明确的保存路径。**Upload** 将一个本地文件上传至当前目录，成功后刷新列表；首版不支持多文件或递归文件夹上传。

底部 **Transfers** 按顺序执行任务，显示 Queued、Running、Succeeded、Failed、Cancelled 状态。**Cancel** 可取消排队或运行中的任务；运行中的子进程先收到 SIGTERM，必要时再终止。应用不删除部分下载文件或 aria2 控制文件。失败或取消后可点击 **Retry**，复用原目标路径及核心提供的续传能力；是否能够续传取决于具体操作与保留的数据，并非所有传输均保证续传。成功下载可通过 Finder 按钮定位。

任务队列仅在本次应用运行期间保留，退出后不恢复任务历史或自动重建队列。需要恢复时重新添加相应任务并使用相同目标路径。界面不提供核心未保证的精确百分比或预计剩余时间。

## 高级设置、扫描与挂载

**Settings** 提供默认下载目录、连接数（1–16）、自定义 IP、SNI、禁用 SNI、自定义 CA、挂载目录与只读挂载选项。自定义 IP 用于列表、下载和上传；SNI、禁用 SNI及自定义 CA 用于下载。连接数用于下载与挂载。留空网络字段使用默认行为。填写 SNI 会关闭“Disable SNI”；启用“Disable SNI”会清空 SNI。加载旧的冲突设置时优先禁用 SNI，下载参数始终保持互斥。

在 **Network & Mount** 中，**Rescan** 重新扫描端点，可填写 IP 地址文件路径（`scan --ips <FILE>`），或使用 **Choose IP File** 选择文件；留空使用默认扫描目标；**List Endpoints** 显示已记录的端点。**Mount** 和 **Unmount** 控制设置中的挂载目录。

挂载要求自行安装 **rclone 1.75.2 或更新版本**，可先在终端运行 `rclone version` 核验。GUI 为 Finder 启动的子进程补充 `/opt/homebrew/bin` 与 `/usr/local/bin` 搜索路径；不会自动安装依赖或调用 sudo。更多挂载条件参见[仓库说明](../README.zh-CN.md)。缺失依赖或操作失败时，界面显示错误信息。

## 固定 CLI 的升级流程

GUI 仅将 `ls --json` 作为结构化协议；其他命令通过参数、退出码与预先确定的目标路径判断结果。升级须作为显式变更审核：

1. 选择明确的官方 release，检查命令参数、`ls --json` 字段、下载输出与取消行为是否兼容；必要时同步更新适配层及契约测试。
2. 修改 `macos/Resources/CLIRelease.json` 的 `version`、`release_url`、`checksums_url` 和两个架构的 `asset`。
3. 从所选官方 release 的 `SHA256SUMS` 核对两个 `.tar.gz` 文件的 SHA256，并写入各自的 `sha256`。可用 `shasum -a 256 /path/to/archive.tar.gz` 复核；不可仅为绕过校验错误而接受未知文件的摘要。构建实际使用本地锁定摘要，`checksums_url` 用于来源记录。
4. 执行下面的测试并分别构建、验证两种架构；在具备授权账号时验证真实登录、浏览、传输和挂载。重新检查最终 APP 内 CLI 的 `--version`，记录未验证项目。

```sh
swift test --package-path macos
python3 -m unittest discover -s macos/Tests -p '*Tests.py'
cargo test --locked
./macos/build-app.sh
macos/dist/GDirect.app/Contents/MacOS/gdirect --version
open macos/dist/GDirect.app
```

Python 测试覆盖打包契约；Swift 测试包括 CLI 适配、模型与队列的模拟成功、失败、取消等情形。模拟测试不能代替真实 Google 授权与网络操作。打包脚本会执行 Swift 测试，不会自动执行上述 Python 或 Rust 测试。

## 签名与分发限制

默认使用 ad-hoc 签名。若已配置 Developer ID，可在构建时指定：

```sh
CODESIGN_IDENTITY='Developer ID Application: Your Name (TEAMID)' ./macos/build-app.sh
```

脚本签名内嵌 CLI、GUI 与 APP，并校验签名；不执行 Apple 公证、上传或发布。ad-hoc 签名及未经公证的 Developer ID 构建均不等同于可直接面向公众分发的公证应用，其他设备可能受到 Gatekeeper 限制。此版本不提供 App Store 沙盒、自动更新或通用双架构包。


CLI 输出在读取阶段受内存上限约束：文件列表 stdout 最多 16 MiB，超过时显示明确错误，不解析被截断的 JSON；其他 stdout 及 stderr 各保留最多 1 MiB，后续输出继续排空以避免阻塞子进程。诊断日志继续进行 URL 查询参数脱敏并按原有上限保留。

若仓库位于 iCloud 同步目录，启动应用或后台同步后，iCloud 可能重新为 `macos/dist/GDirect.app` 添加 `com.apple.FinderInfo`，导致后续严格签名校验失败；即使执行 `xattr -rc`，同步也可能再次添加该属性。此环境下，可将构建结果复制到未同步的本地目录，再校验并启动该副本。以下命令创建独立临时目录，避免覆盖已有应用；应确认系统临时目录未被同步：

```sh
local_app_dir=$(mktemp -d "${TMPDIR:-/tmp}/gdirect-local.XXXXXX")
ditto --norsrc macos/dist/GDirect.app "$local_app_dir/GDirect.app"
python3 macos/Scripts/verify-app.py "$local_app_dir/GDirect.app" --version 0.2.0 --arch "$(uname -m)"
open "$local_app_dir/GDirect.app"
```
