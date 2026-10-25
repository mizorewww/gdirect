# 配置 Google

[English](README.md) · [返回 README](../../README.zh-CN.md)

gdirect 使用你在自己的 Google Cloud 项目里创建的 OAuth 客户端登录。只需配置一次，大约 10 分钟，Google 不收费。gdirect 不会分发或使用任何他人的客户端。

## 1. 创建桌面 OAuth 客户端

请按照[创建 Google OAuth 客户端](google-cloud.zh-CN.md)操作，文中配有每一步的截图。完成后你会得到：

- 一个已启用 Drive API 的 Google Cloud 项目；
- 一个已添加 Drive 权限范围、并允许你的账号登录的 OAuth 应用；
- 一个已下载的 **Desktop app（桌面应用）** 客户端 JSON。

## 2. 登录

在终端执行下面的命令，把路径换成你下载的 JSON 文件：

```sh
gdirect login --client-secret ~/Downloads/client_secret.json
```

在 Windows PowerShell 中，路径写成 `"$HOME\Downloads\client_secret.json"`，同样换成你实际下载的文件名。

1. 程序会在这台电脑上打开浏览器。如果没有自动打开，请把终端里打印的链接复制到**同一台电脑**的浏览器中。授权完成后会回调到本机的临时端口，所以不能在手机或其他电脑上打开。
2. 选择刚才添加为测试用户的 Google 账号。
3. Google 会提示此应用未经验证。这是正常的，因为这是你自己创建的应用。
   - 测试状态：点击 **继续（Continue）**，不要点击 **返回到安全网页（Back to safety）**。
   - 正式版：点击 **Advanced（高级）**，再点击 **Go to 你的应用名 (unsafe)（转至……（不安全））**。

   ![未验证应用提示](images/12-unverified.png)

4. 核对应用名称和 Google 云端硬盘权限，确认无误后点击 **继续（Continue）**。

   ![确认 Drive 授权](images/13-consent.png)

5. 回到终端，等待显示 `Logged in`。

登录完成后可以删除下载的 JSON 文件。gdirect 已保存了一份客户端信息，以后执行 `gdirect login --reauth` 时会用到。

检查是否可用：

```sh
gdirect ls root     # 列出云端硬盘根目录
gdirect browse
```

## 常见问题

| 现象 | 处理方法 |
| --- | --- |
| `access_denied` 或提示应用正在测试 | 把当前账号加为测试用户，或发布应用（见[第 5 步](google-cloud.zh-CN.md#5-允许你的账号登录)） |
| `redirect_uri_mismatch` | 这个 JSON 不是 **Desktop app** 客户端，按[第 4 步](google-cloud.zh-CN.md#4-创建桌面客户端并下载-json)重新创建 |
| `Drive API 403 … insufficient permission` | 在客户端所在的**同一个项目**中启用 Drive API（[第 1 步](google-cloud.zh-CN.md#1-新建项目并启用-drive-api)），并保存 Drive 权限范围（[第 3 步](google-cloud.zh-CN.md#3-添加-drive-权限范围)）；然后执行 `gdirect login --reauth` 并允许访问云端硬盘 |
| `refresh credential expired or revoked` | 执行 `gdirect login --reauth`。如果每 7 天出现一次，说明应用仍处于测试状态（见[第 5 步](google-cloud.zh-CN.md#5-允许你的账号登录)） |
| 登录超时或报网络错误 | 登录需要访问 `accounts.google.com`、`oauth2.googleapis.com` 和 `www.googleapis.com`，见[只开系统代理，不开 TUN](../../README.zh-CN.md#只开系统代理不开-tun) |
| 登错账号或需要切换账号 | 执行 `gdirect login --reauth`，选择另一个账号 |

- **挂载与新凭据：** 正在运行的挂载会继续使用旧凭据。执行 `--reauth` 后，请先卸载再重新挂载。
- **无界面服务器：** 浏览器登录要求在同一台机器上打开浏览器。可以先在桌面电脑上登录，或者用 `gdirect login --credentials 路径` 导入已有的 `authorized_user` 凭据 JSON。Google Cloud SDK 生成的应用默认凭据（ADC）通常不包含 Drive 权限。
- **退出登录：** `gdirect logout` 会删除已保存的凭据。也可以在 [myaccount.google.com/permissions](https://myaccount.google.com/permissions) 撤销授权。
- **妥善保管：** 客户端 JSON、凭据 JSON 和 SQLite 数据库都要像密码一样对待，不要提交到 Git，也不要分享给别人。

Google 官方文档：[桌面应用 OAuth](https://developers.google.com/identity/protocols/oauth2/native-app)、[刷新令牌的过期规则](https://developers.google.com/identity/protocols/oauth2#expiration)。
