# GDirect macOS 图形界面设计

## 目标与边界

为现有 gdirect 增加简洁、原生的 macOS 图形界面，并提供一条命令构建可双击启动的 `.app`。GUI 独立于上游 Rust 内部实现：不引入 Rust FFI，不直接读写 SQLite，不依赖私有函数签名，不自动跟随上游 main。

首版包含账号登录/重新授权/注销、Drive 文件浏览、文件与公开链接下载、Google 文档导出、单文件上传、顺序任务队列与取消/重试、挂载/卸载、IP 扫描及高级路由设置。非目标：App Store 沙盒、自动更新 CLI、iOS 版本、多文件/递归文件夹上传、核心未提供的可靠百分比/ETA。

## 已选方案及取舍

采用独立 SwiftUI shell + 固定版本 CLI。`ls --json` 是唯一结构化数据协议；其他操作以参数、退出码和已知目标路径判断结果。GUI 集中封装全部命令，固定 CLI 版本与发布包 SHA256，升级核心时显式修改版本并跑契约测试。

未采用 Rust FFI：当前没有公共 Rust 库，直接绑定会把 GUI 编译与私有模块、aria2/OpenSSL 工具链耦合。未采用 Tauri：会增加前端/Rust 依赖，且不能天然解决核心接口变化。

## 应用结构与体验

所有新代码放在顶层 `macos/` 独立目录，使用无第三方依赖的 Swift Package（SwiftUI + AppKit），macOS 15+，本机 arm64/x86_64。`swift build` 生成 GUI 可执行文件，脚本组装标准 `GDirect.app`，不要求手动操作 Xcode。

单窗口顶部显示账号状态与登录/设置入口。主区域为 Drive 文件列表，文件夹优先，支持进入、返回、刷新，展示名称、类型、大小、修改时间。选中文件后可下载；Google Docs/Sheets/Slides 默认导出 PDF，并可修改 MIME；支持上传到当前目录及通过链接下载公开文件。

任务区域展示排队、运行、成功、失败、已取消，支持取消运行任务、重试、完成后在 Finder 中显示。任务只在应用运行期间保留；取消不删除部分文件，重试复用核心续传能力。

设置页提供默认下载目录、连接数 1–16、自定义 IP、SNI/禁用 SNI、自定义 CA、挂载目录和只读选项。网络/挂载区域支持重新扫描、查看端点、挂载和卸载；缺少 rclone 1.75.2+ 时提示安装。加载使用原生进度指示和空状态，错误以内联横幅显示，保持 macOS 原生简洁风格。

## CLI 隔离与打包契约

- `macos/Package.swift`、`Sources/GDirect/`、`Tests/GDirectTests/`、`Resources/`、`build-app.sh` 和 `README.md` 自包含；核心 `src/`、根 Cargo 配置与现有发布脚本不改。
- 一个 CLI 适配层负责定位内嵌二进制、构造参数、分离 stdout/stderr、管理子进程及退出状态；SwiftUI 不直接拼命令。
- `DriveFile` 解码 `id/name/mimeType/size/modifiedTime/md5Checksum/resourceKey`；可选字段保持可选，忽略新增字段。
- 只有目录列表解析 JSON。其他命令用退出码判断；上传成功后刷新当前目录；下载完成使用 GUI 预先确定的输出路径，不解析英文成功文案。
- 日志仅作瞬态诊断，内存限长；不持久化 OAuth URL、token、上传 session 或完整命令环境。
- 凭据继续只由 CLI 写入私有 SQLite；GUI 的 UserDefaults 仅保存非敏感偏好。
- 子进程取消使用 SIGTERM，必要时延迟升级为 SIGKILL；不删除输出或 aria2 控制文件。
- Finder 启动时补充常见 Homebrew 路径以找到 `rclone`，不自动安装、不调用 sudo。
- 默认 `./macos/build-app.sh` 根据本机架构下载固定版本官方 release，核对锁定 SHA256；`--cli /absolute/path/to/gdirect` 为显式本地调试入口。
- APP 的 `Contents/MacOS/` 同时放置 GUI、内嵌 `gdirect` 与 `cacert.pem`。写入 `Info.plist`，默认 ad-hoc 签名；可通过 `CODESIGN_IDENTITY` 使用 Developer ID，公证不在范围内。
- 输出 `macos/dist/GDirect.app`。构建脚本执行测试、检查 CLI 版本/架构、CA、plist 与最终包结构。

## 验收标准

- 在干净终端运行 `./macos/build-app.sh` 得到可启动 APP，不修改核心源码。
- 浏览兼容当前 JSON；传输/挂载失败可见；取消与重试安全；任务不阻塞 UI。
- 敏感信息不写入 GUI 持久日志；构建包包含匹配架构的 CLI 和 CA。
- 核心测试不被破坏；新增测试及 APP 冒烟检查通过。真实 Google 操作仅在具备授权账号时验证。
