# 创建 Google OAuth 客户端

[English](google-cloud.md) · [返回 Google 配置](README.zh-CN.md)

本文带你在 Google Cloud 控制台中一步一步操作，最终得到 gdirect 需要的 **Desktop app（桌面应用）** OAuth 客户端 JSON。只需配置一次，大约 10 分钟，Google 不收费。

开始前请准备好一个 Google 账号和浏览器。下面的截图来自中文界面，每张图下方有中英文说明，带编号的框就是要操作的位置。

## 1. 新建项目并启用 Drive API

1. 打开 [Google Cloud 控制台](https://console.cloud.google.com/)，点击顶部的项目选择器，再点击 **New project（新建项目）**。名称随意，例如 `gdirect`。个人账号的 **Location（位置）** 保持 **No organization（无组织）** 即可。点击 **Create（创建）** 后，确认页面顶部已经切换到这个新项目。后面所有步骤都在这个项目里完成。

   ![新建项目](images/01-project.png)

2. 打开 [Google Drive API](https://console.cloud.google.com/apis/library/drive.googleapis.com) 页面，确认顶部是刚才的项目，然后点击 **Enable（启用）**。

   ![启用 Drive API](images/02-enable-api.png)

   完成后，页面会显示服务名称 `drive.googleapis.com`，状态为 **Enabled（已启用）**。

   ![Drive API 已启用](images/03-api-enabled.png)

## 2. 配置 OAuth 应用

1. 打开 [Google Auth Platform](https://console.cloud.google.com/auth/overview)，点击 **Get started（开始）**。
2. **App information（应用信息）**：填写应用名称（登录时 Google 会显示这个名字），用户支持邮箱选择你自己的邮箱，点击 **Next（下一步）**。

   ![应用名称和支持邮箱](images/04-branding.png)

3. **Audience（受众群体）**：选择 **External（外部）**，点击 **Next（下一步）**。个人账号不能选择 Internal（内部）。

   ![选择外部](images/05-audience.png)

4. **Contact information（联系信息）**：填写你的邮箱，点击 **Next（下一步）**。
5. **Finish（完成）**：勾选同意 Google API 服务用户数据政策，点击 **Continue（继续）**，再点击 **Create（创建）**。

   ![同意数据政策](images/06-policy.png)

## 3. 添加 Drive 权限范围

1. 在左侧菜单打开 **Data Access（数据访问）**，点击 **Add or remove scopes（添加或移除范围）**。
2. 在弹出的面板中勾选 `.../auth/drive`（说明为“查看、修改、创建和删除您的所有 Google 云端硬盘文件”）。如果列表里找不到，就把下面的地址粘贴到 **Manually add scopes（手动添加范围）** 中，点击 **Add to table（添加到表）**。然后点击 **Update（更新）**。

   ```text
   https://www.googleapis.com/auth/drive
   ```

   ![选择 Drive 权限范围](images/07-add-scope.png)

3. 回到数据访问页面后，点击底部的 **Save**。只点 **Update（更新）** 并不会保存。

   ![保存 Drive 权限范围](images/08-scope-saved.png)

gdirect 需要这项完整的云端硬盘权限，才能浏览已有文件、下载、上传，以及把云端硬盘挂载成可写的磁盘。

## 4. 创建桌面客户端并下载 JSON

1. 在左侧菜单打开 **Clients（客户端）**，点击 **Create client（创建客户端）**。应用类型选择 **Desktop app（桌面应用）**，名称随意，例如 `gdirect Desktop`，然后点击 **Create（创建）**。

   ![创建桌面应用客户端](images/09-desktop-client.png)

2. 在弹出的窗口中，**关闭之前**复制 **Client ID（客户端 ID）** 和 **Client secret（客户端密钥）**，或者点击 **Download JSON（下载 JSON）**。密钥只在这个窗口里显示，关闭后就无法再查看或下载（找回方法见下文的“添加新密钥”）。

   ![下载客户端 JSON](images/10-client-created.png)

只能使用 **Desktop app（桌面应用）** 客户端。网页客户端、API Key 和服务账号 JSON 都不能用。

<a id="client-id-and-secret"></a>

### 复制客户端 ID 和密钥

gdirect 需要客户端的两个值。可以在 gdirect 的登录表单里逐项填写或粘贴；也可以把下载的整个 JSON（或者 ID 和密钥一起）粘贴到“客户端 ID”框里，两项会自动填好。桌面客户端的“从 JSON 文件导入…”可以直接读取下载的文件。

- **客户端 ID** 以 `.apps.googleusercontent.com` 结尾。创建时的窗口里会显示；之后也可以随时复制：打开 [Google Auth Platform › 客户端](https://console.cloud.google.com/auth/clients)，点开你的桌面应用客户端即可。
- **客户端密钥** 通常以 `GOCSPX-` 开头。创建时的窗口里会显示，下载的 JSON 中也有。如果关闭窗口时没有保存，可以在[客户端](https://console.cloud.google.com/auth/clients)列表中点开这个客户端，在 **Client secrets（客户端密钥）** 下点击 **Add secret（添加密钥）**，立即复制新密钥；之后可以停用或删除旧密钥。

## 5. 允许你的账号登录

新建的应用处于 **Testing（测试）** 状态，只有测试用户列表中的账号可以登录。

1. 在左侧菜单打开 **Audience（目标对象）**，在 **Test users（测试用户）** 下点击 **Add users**，填入要在 gdirect 中使用的 Google 账号邮箱，点击 **Save（保存）**。确认这个账号已经出现在列表中。

   ![添加测试用户](images/11-test-user.png)

测试状态下，登录 **7 天** 后会失效，到时执行 `gdirect login --reauth` 重新登录即可。

如果不想每周重新登录，可以在目标对象页面顶部点击 **Publish app（发布应用）**，把状态改为 **In production（正式版）**。自己使用的应用不需要经过 Google 审核。如果这个按钮是灰色的，并提示需要完成品牌配置，请先在 **Branding（品牌塑造）** 页面填好必填项；也可以直接保持测试状态。

## 下一步

客户端 ID 和密钥（或 JSON）已经准备好了，接下来[用 gdirect 登录](README.zh-CN.md#2-登录)。
