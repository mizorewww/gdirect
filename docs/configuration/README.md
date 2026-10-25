# Google setup

[中文](README.zh-CN.md) · [Back to README](../../README.md)

gdirect signs in with an OAuth client that you create in your own Google Cloud project. You only do this once, and it takes about 10 minutes. Google does not charge for it, and gdirect never ships or uses anyone else's client.

## 1. Create a Desktop OAuth client

Follow [Create a Google OAuth client](google-cloud.md). It walks through the Google Cloud console with screenshots. When you finish, you will have:

- a Google Cloud project with the Drive API enabled;
- an OAuth app with the Drive scope, and your account allowed to sign in;
- a downloaded **Desktop app** client JSON.

## 2. Sign in

Run this in a terminal, replacing the path with the JSON file you downloaded:

```sh
gdirect login --client-secret ~/Downloads/client_secret.json
```

On Windows PowerShell, write the path as `"$HOME\Downloads\client_secret.json"`, again with your actual file name.

1. A browser opens on this computer. If it does not, copy the URL printed in the terminal into a browser **on the same computer**. Google sends the sign-in back to a temporary local port, so another device will not work.
2. Choose the Google account you added as a test user.
3. Google warns that it has not verified the app. This is expected, because the app is your own.
   - In Testing, click **Continue**. Do not click **Back to safety**.
   - In production, click **Advanced**, then **Go to *your app name* (unsafe)**.

   ![Unverified-app notice](images/12-unverified.png)

4. Check that the app name and the Google Drive permission are what you expect, then click **Continue**.

   ![Allow Drive access](images/13-consent.png)

5. Return to the terminal and wait for `Logged in`.

You can now delete the downloaded JSON. gdirect keeps a private copy of the client for `gdirect login --reauth`.

Check that it works:

```sh
gdirect ls root     # lists the top of your Drive
gdirect browse
```

## Troubleshooting

| Problem | Fix |
| --- | --- |
| `access_denied` or "app is being tested" | Add this account as a test user, or publish the app ([step 5](google-cloud.md#5-allow-your-account-to-sign-in)) |
| `redirect_uri_mismatch` | The JSON is not a **Desktop app** client. Create one as in [step 4](google-cloud.md#4-create-a-desktop-client-and-download-its-json) |
| `Drive API 403 … insufficient permission` | Enable the Drive API in the **same** project as the client ([step 1](google-cloud.md#1-create-a-project-and-enable-the-drive-api)) and save the Drive scope ([step 3](google-cloud.md#3-add-the-drive-scope)). Then run `gdirect login --reauth` and allow Drive access |
| `refresh credential expired or revoked` | Run `gdirect login --reauth`. If this happens every 7 days, the app is still in Testing ([step 5](google-cloud.md#5-allow-your-account-to-sign-in)) |
| Sign-in times out or shows a network error | Sign-in needs access to `accounts.google.com`, `oauth2.googleapis.com` and `www.googleapis.com`; see [System proxy without TUN](../../README.md#system-proxy-without-tun) |
| Wrong account, or switching accounts | `gdirect login --reauth`, then choose the other account |

- **Mounts and new credentials:** a running mount keeps its old credentials. After `--reauth`, unmount and mount again.
- **Headless servers:** browser sign-in needs a browser on the same machine. Sign in on a desktop, or import an existing `authorized_user` credentials JSON with `gdirect login --credentials PATH`. Google Cloud SDK application-default credentials usually lack the Drive scope.
- **Logging out:** `gdirect logout` deletes the stored credentials. You can also revoke access at [myaccount.google.com/permissions](https://myaccount.google.com/permissions).
- **Keep files private:** treat the client JSON, credentials JSON and the SQLite database like passwords. Do not commit or share them.

Google's reference: [OAuth for desktop apps](https://developers.google.com/identity/protocols/oauth2/native-app), [refresh token expiration](https://developers.google.com/identity/protocols/oauth2#expiration).
