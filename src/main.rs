use anyhow::{Context, Result, ensure};
use clap::{Args, Parser, Subcommand};
use gdirect::{api, download, drive, mount, netif, ranges, route, scan, store, tui, upload};
use std::{
    net::IpAddr,
    path::{Path, PathBuf},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Parser)]
#[command(version, about = "Browse, download and upload Google Drive files")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
    /// Local SQLite state file (default: platform data directory)
    #[arg(long, global = true)]
    db: Option<PathBuf>,
    /// Signed-in account to use: an address or an unambiguous prefix (default: see gdirect accounts)
    #[arg(short = 'a', long, global = true, env = "GDIRECT_ACCOUNT")]
    account: Option<String>,
    /// No effect: only Google's own front ends are used (kept so scripts that
    /// pass it keep working)
    #[arg(
        long,
        global = true,
        hide = true,
        env = "GDIRECT_GOOGLE_ONLY",
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    google_only: bool,
}
#[derive(Args, Clone, Default)]
struct Route {
    /// Connect to this Google IP instead of the best scanned one
    #[arg(long)]
    ip: Option<IpAddr>,
    /// TLS identity hostname to use instead of the original Google hostname
    /// (the Host header still carries the real Google hostname)
    #[arg(long)]
    sni: Option<String>,
    /// Send no TLS SNI (downloads only); the certificate is still verified
    #[arg(long, conflicts_with = "sni")]
    no_sni: bool,
}
impl Route {
    /// Explicit identity choice: Some(name) to forge, Some("") for no SNI,
    /// None to follow the scanned route and allow automatic fallback.
    fn sni(&self) -> Option<String> {
        if self.no_sni {
            Some(String::new())
        } else {
            self.sni.clone()
        }
    }
}
#[derive(Args, Clone, Default)]
struct Transfer {
    #[command(flatten)]
    route: Route,
    /// CA bundle for certificate verification (default: bundled Google GTS roots)
    #[arg(long)]
    ca_cert: Option<PathBuf>,
    /// Connections per download, 1-16 (default: value saved in browse, else 8)
    #[arg(short = 'j', long, value_parser = clap::value_parser!(u8).range(1..=16))]
    connections: Option<u8>,
}
#[derive(Subcommand, Clone)]
enum Commands {
    /// Probe Google endpoints and save working IPs in SQLite
    Scan {
        /// Probe only the IPs in this file (one per line, # starts a comment)
        #[arg(long, conflicts_with = "list")]
        ips: Option<PathBuf>,
        /// Show saved endpoints instead of scanning
        #[arg(long)]
        list: bool,
        /// Bind Google connections to the network adapter so TUN-mode proxies
        /// and VPNs cannot capture them: on (default) or off to let proxy
        /// rules decide. Saved for later commands and the desktop client;
        /// GDIRECT_BYPASS_TUN=0/1 overrides it for one run
        #[arg(
            long,
            value_name = "on|off",
            hide_possible_values = true,
            value_parser = clap::builder::BoolishValueParser::new()
        )]
        bypass_tun: Option<bool>,
    },
    /// Sign in with a Google Desktop OAuth client JSON; signing in again adds another account
    Login {
        /// Sign in in the browser using the saved OAuth client; the account you
        /// choose there is added (or renewed) and becomes the default
        #[arg(long, visible_alias = "add")]
        reauth: bool,
        /// Google Desktop OAuth client JSON (opens browser)
        #[arg(long, conflicts_with = "credentials")]
        client_secret: Option<PathBuf>,
        /// Import existing authorized_user credentials JSON (e.g. on a headless server)
        #[arg(long, conflicts_with = "reauth")]
        credentials: Option<PathBuf>,
    },
    /// Remove an account's saved credentials (default: the default account); scanned IPs are kept
    Logout {
        /// Account to sign out: an address or an unambiguous prefix
        account: Option<String>,
        /// Sign out every account
        #[arg(long, conflicts_with = "account")]
        all: bool,
    },
    /// List signed-in accounts; * marks the default
    Accounts,
    /// Make another signed-in account the default
    Switch {
        /// Address or unambiguous prefix
        account: String,
    },
    /// Browse and manage folders; ? lists the keys (d download, a create, r rename, D trash)
    Browse {
        /// Folder ID to open
        #[arg(default_value = "root")]
        folder: String,
        /// Download directory
        #[arg(short, long, default_value = "downloads")]
        output: PathBuf,
        #[command(flatten)]
        transfer: Transfer,
    },
    /// List a Drive folder
    Ls {
        /// Folder ID to list
        #[arg(default_value = "root")]
        folder: String,
        /// Print JSON instead of tab-separated columns
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        route: Route,
    },
    /// Mount Drive locally using rclone (macOS NFS; Linux/Windows FUSE)
    Mount {
        /// Mount directory (default: ~/GoogleDrive)
        path: Option<PathBuf>,
        /// Mount without write access
        #[arg(long)]
        read_only: bool,
        /// Stay in the foreground and stop with Ctrl-C (always on Windows)
        #[arg(long)]
        foreground: bool,
        /// Parallel rclone transfers, 1-16 (default: value saved in browse, else 8)
        #[arg(short = 'j', long, value_parser = clap::value_parser!(u8).range(1..=16))]
        connections: Option<u8>,
        #[command(flatten)]
        route: Route,
    },
    /// Runs rclone for a background mount behind a proxy pinned to `endpoint`s
    #[command(hide = true)]
    MountHelper {
        #[arg(long = "endpoint", required = true)]
        endpoints: Vec<IpAddr>,
        #[arg(last = true, required = true)]
        command: Vec<std::ffi::OsString>,
    },
    /// Unmount a background mount (default: the default account's, or the only one)
    Unmount,
    /// Upload a local file, or a folder with --recursive (repeat the command to resume)
    Upload {
        /// Local file, or folder with --recursive
        file: PathBuf,
        /// Upload a folder and everything inside it
        #[arg(short = 'r', long)]
        recursive: bool,
        /// Destination folder ID
        #[arg(long, default_value = "root")]
        parent: String,
        /// Name in Drive (default: the local file or folder name)
        #[arg(long)]
        name: Option<String>,
        /// Start a recursive upload over instead of resuming an interrupted one
        #[arg(long, requires = "recursive")]
        restart: bool,
        #[command(flatten)]
        route: Route,
    },
    /// Download a file ID or sharing URL (repeat the command to resume)
    Download {
        /// Drive file ID or sharing URL
        file: String,
        /// Output file (default: downloads/<name> when logged in; required for public files)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Export a Google document as this MIME type, e.g. application/pdf
        #[arg(long)]
        export: Option<String>,
        #[command(flatten)]
        transfer: Transfer,
    },
}
/// One line on how connections leave this machine, for `scan --list`.
fn network_summary(status: &netif::Status) -> String {
    let tun = status
        .tun
        .as_deref()
        .map(|tun| format!(", past the TUN interface {tun}"))
        .unwrap_or_default();
    match (status.bound(), &status.physical) {
        (Some(bound), _) => format!(
            "Network: bypassing TUN proxies, bound to {}{tun}",
            bound.name
        ),
        (None, Some(physical)) if status.enabled => format!(
            "Network: bypassing TUN proxies, but binding to {} is not allowed ({}){tun}",
            physical.name,
            status.denied.as_deref().unwrap_or("unknown")
        ),
        _ if status.enabled => "Network: bypassing TUN proxies; no network adapter found".into(),
        _ => match &status.tun {
            Some(tun) => {
                format!("Network: not bypassing TUN proxies; {tun} carries the default route")
            }
            None => "Network: not bypassing TUN proxies (none detected)".into(),
        },
    }
}
fn direct_route(db: &store::Store, route: &Route) -> Result<route::Direct> {
    route::resolve(db, route.ip, route.sni())
}
fn run(cli: Cli) -> Result<()> {
    let db_path = cli.db.clone().map(Ok).unwrap_or_else(store::default_path)?;
    let db = store::Store::open(&db_path)?;
    if let Some(Commands::Scan {
        bypass_tun: Some(on),
        ..
    }) = &cli.command
    {
        netif::save(&db, *on)?;
        if std::env::var_os(netif::ENV).is_some() {
            eprintln!(
                "note: {} is set and overrides this for each run",
                netif::ENV
            );
        }
    }
    // Applies the TUN bypass choice and retires endpoints verified over
    // another network path, so even `scan --list` shows what still counts.
    netif::sync(&db)?;
    // Commands that work without a login resolve the account lazily.
    let account = || db.resolve_account(cli.account.as_deref());
    let saved = || -> u8 {
        db.setting("connections")
            .ok()
            .flatten()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8)
            .clamp(1, 16)
    };
    let command = cli.command.unwrap_or_else(|| Commands::Browse {
        folder: "root".into(),
        output: "downloads".into(),
        transfer: Transfer::default(),
    });
    match command {
        Commands::Scan { list: true, .. } => {
            eprintln!("{}", network_summary(&netif::status()));
            let google = ranges::current(&db);
            for e in db.endpoints()? {
                // Stored by older releases; transfers never use it.
                let owner = match e.ip.parse() {
                    Ok(ip) if google.contains(ip) => "",
                    _ => " [not Google; ignored]",
                };
                match e.latency_ms {
                    Some(ms) if e.sni.is_empty() => println!("{}\tOK\t{ms} ms{owner}", e.ip),
                    Some(ms) => println!("{}\tOK\t{ms} ms (SNI {}){owner}", e.ip, e.sni),
                    None => println!(
                        "{}\tunavailable\t{}{owner}",
                        e.ip,
                        e.error.unwrap_or_default()
                    ),
                }
            }
        }
        Commands::Scan { ips, .. } => {
            scan::run(&db, ips.as_deref())?;
        }
        Commands::Unmount => {
            let asked = match cli.account.as_deref() {
                Some(_) => account()?,
                None => None,
            };
            mount::unmount(&db, asked.as_deref())?
        }
        Commands::Logout { account: name, all } => {
            logout(&db, all, name.as_deref().or(cli.account.as_deref()))?
        }
        Commands::Accounts => {
            let default = db.default_account()?;
            let accounts = db.accounts()?;
            if accounts.is_empty() {
                println!("No account signed in; run gdirect login");
            }
            for email in accounts {
                let mark = if Some(&email) == default.as_ref() {
                    '*'
                } else {
                    ' '
                };
                println!("{mark} {email}");
            }
        }
        Commands::Switch { account: name } => {
            let email = db
                .resolve_account(Some(&name))?
                .context("no account signed in; run gdirect login")?;
            db.set_default_account(&email)?;
            println!("Default account: {email}");
        }
        Commands::Mount {
            path,
            read_only,
            foreground,
            connections,
            route,
        } => {
            let connections = connections.unwrap_or_else(saved);
            // rclone reaches Google through the verified endpoints; when a scan
            // finds none, it falls back to the system network as before.
            let (api, endpoints) = match direct_route(&db, &route) {
                Ok(direct) => (direct.api(db_path, account()?)?, direct.tunnel_endpoints()),
                Err(error) if route.ip.is_some() => return Err(error),
                Err(error) => {
                    eprintln!("warning: {error:#}; mounting over the system network");
                    (
                        api::Api::new(db_path, account()?, None, None, false)?,
                        Vec::new(),
                    )
                }
            };
            mount::run(
                &api,
                path.as_deref(),
                read_only,
                foreground,
                connections,
                endpoints,
            )?;
        }
        Commands::MountHelper { endpoints, command } => {
            mount::helper(endpoints, &command)?;
        }
        Commands::Login {
            reauth,
            client_secret,
            credentials,
        } => {
            // The very first login has no account to pick yet.
            let current = if db.accounts()?.is_empty() {
                None
            } else {
                account()?
            };
            let api = api::Api::new(db_path, current, None, None, false)?;
            let email = api.login(reauth, client_secret.as_deref(), credentials.as_deref())?;
            println!(
                "Logged in: {email} (default account)\nSQLite: {}",
                api.db.display()
            );
            if !reauth && client_secret.is_none() && credentials.is_none() {
                println!("To add another account: gdirect login --add");
            }
        }
        Commands::Ls {
            folder,
            json,
            route,
        } => {
            let account = account()?;
            let api = direct_route(&db, &route)?.api(db_path, account)?;
            let files = api.list(&folder)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&files)?);
            } else {
                for f in files {
                    println!(
                        "{}\t{}\t{}\t{}",
                        if f.folder() { "DIR" } else { "FILE" },
                        f.id,
                        f.size.as_deref().unwrap_or("-"),
                        f.name
                    );
                }
            }
        }
        Commands::Upload {
            file,
            recursive,
            parent,
            name,
            restart,
            route,
        } => {
            // Check the local path before scanning for endpoints, so a typo or a
            // missing --recursive fails immediately.
            let metadata = std::fs::metadata(&file)
                .with_context(|| format!("cannot open {}", file.display()))?;
            if metadata.is_dir() {
                ensure!(
                    recursive,
                    "{} is a folder; add --recursive to upload it and its contents",
                    file.display()
                );
            } else {
                ensure!(
                    !recursive,
                    "--recursive expects a folder, but {} is not a folder",
                    file.display()
                );
            }
            let account = account()?;
            let api = direct_route(&db, &route)?.api(db_path, account)?;
            if metadata.is_dir() {
                let uploaded = upload_folder(&api, &file, &parent, name.as_deref(), restart)?;
                eprintln!("{}", uploaded.summary());
                println!("{}\t{}", uploaded.id, uploaded.name);
            } else {
                let uploaded = api.upload(&file, &parent, name.as_deref())?;
                println!("{}\t{}", uploaded.id, uploaded.name);
            }
        }
        Commands::Download {
            file,
            output,
            export,
            transfer,
        } => {
            let account = account()?;
            let api = direct_route(&db, &transfer.route)?.api(db_path, account)?;
            let connections = transfer.connections.unwrap_or_else(saved);
            start_download(&api, file, output, export, &transfer, connections)?;
        }
        Commands::Browse {
            folder,
            output,
            transfer,
        } => {
            let account = account()?;
            let api = direct_route(&db, &transfer.route)?.api(db_path, account)?;
            let options = tui::Options {
                folder,
                output,
                no_sni: transfer.route.no_sni,
                auto: transfer.route.sni().is_none(),
                connections: transfer.connections.unwrap_or_else(saved),
                ca_cert: transfer.ca_cert,
            };
            tui::run(api, options)?;
        }
    }
    Ok(())
}
fn logout(db: &store::Store, all: bool, name: Option<&str>) -> Result<()> {
    let accounts = if all {
        db.accounts()?
    } else {
        db.resolve_account(name)?.into_iter().collect()
    };
    if accounts.is_empty() {
        println!("No saved login; nothing removed.");
        return Ok(());
    }
    for account in &accounts {
        anyhow::ensure!(
            db.setting(&store::Store::mount_key(account))?.is_none(),
            "{account} has a background mount; run gdirect unmount --account {account} first"
        );
    }
    for account in &accounts {
        db.remove_account(account)?;
        println!(
            "Logged out {account}: removed saved credentials, access token and upload sessions."
        );
    }
    match db.default_account()? {
        Some(next) => {
            db.set_default_account(&next)?;
            println!("Default account: {next}");
        }
        None => {
            println!("No account left; the saved OAuth client was removed. Scanned IPs are kept.")
        }
    }
    Ok(())
}
/// Uploads a folder recursively, drawing one updating line per file.
fn upload_folder(
    api: &api::Api,
    path: &Path,
    parent: &str,
    name: Option<&str>,
    restart: bool,
) -> Result<upload::TreeUpload> {
    eprintln!("Scanning {}…", path.display());
    let mut current = String::new();
    let result = api.upload_dir(path, parent, name, restart, |progress| {
        if progress.path != current {
            if !current.is_empty() {
                eprintln!();
            }
            current = progress.path.clone();
        }
        let percent = upload::percent(progress.file_done, progress.file_total);
        eprint!(
            "\r[{}/{}] {} · {percent}%",
            progress.files_done,
            progress.files_total,
            tail(&progress.path, 56)
        );
    });
    if !current.is_empty() {
        eprintln!();
    }
    result
}
/// Keeps the end of a long relative path, where the file name is.
fn tail(text: &str, max: usize) -> String {
    if text.width() <= max {
        return text.to_owned();
    }
    let mut kept = Vec::new();
    let mut width = 0;
    for c in text.chars().rev() {
        let c_width = c.width().unwrap_or(0);
        // One column stays for the leading ellipsis.
        if width + c_width + 1 > max {
            break;
        }
        kept.push(c);
        width += c_width;
    }
    kept.reverse();
    format!("…{}", kept.into_iter().collect::<String>())
}
fn start_download(
    api: &api::Api,
    file: String,
    output: Option<PathBuf>,
    export: Option<String>,
    transfer: &Transfer,
    connections: u8,
) -> Result<()> {
    let token = if api.account.is_some() || std::env::var_os("GDIRECT_ACCESS_TOKEN").is_some() {
        Some(api.download_token()?)
    } else {
        None
    };
    let id = drive::FileRef::parse(&file)?.id;
    let metadata = if token.is_some() {
        Some(api.metadata(&id)?)
    } else {
        None
    };
    if let Some(meta) = &metadata {
        anyhow::ensure!(
            !meta.folder(),
            "select a file inside the folder with gdirect browse"
        );
        anyhow::ensure!(
            meta.downloadable(),
            "{} has no downloadable content",
            meta.mime_type
        );
        anyhow::ensure!(
            !meta.workspace() || export.is_some(),
            "this is a Google document; add --export application/pdf (or another export MIME type)"
        );
    }
    let output = output
        .or_else(|| {
            metadata
                .as_ref()
                .map(|meta| api::destination(std::path::Path::new("downloads"), meta))
        })
        .context("public download requires -o FILE")?;
    // Read after the metadata request, which may already have failed over.
    let (ip, sni, backups) = api.route();
    download::run(
        download::Args {
            file,
            output,
            ip,
            sni: if transfer.route.no_sni {
                Some(String::new())
            } else {
                sni
            },
            auto: transfer.route.sni().is_none(),
            backups,
            db: Some(api.db.clone()),
            md5: metadata
                .as_ref()
                .filter(|_| export.is_none())
                .and_then(|meta| meta.md5_checksum.clone()),
            token,
            export,
            connections,
            ca_cert: transfer.ca_cert.clone(),
        },
        None,
    )
}
fn main() {
    if let Err(e) = run(Cli::parse()) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("gdirect").chain(args.iter().copied()))
    }
    #[test]
    fn commands_and_scoped_options() {
        let cli = parse(&["browse", "-j", "8"]).unwrap();
        assert!(
            matches!(cli.command, Some(Commands::Browse { transfer, .. }) if transfer.connections == Some(8))
        );
        assert!(parse(&["browse", "-j", "0"]).is_err());
        assert!(parse(&[]).unwrap().command.is_none());
        assert!(parse(&["lgoin"]).is_err());
        assert!(parse(&["file_id", "-o", "out.bin"]).is_err());
        assert!(parse(&["--auth", "ls"]).is_err());
        assert!(parse(&["scan", "--list", "--db", "x.sqlite3"]).is_ok());
        assert!(parse(&["login", "--ip", "1.1.1.1"]).is_err());
        assert!(parse(&["scan", "-o", "x"]).is_err());
        assert!(parse(&["ls", "--sni", "g.cn"]).is_ok());
        assert!(parse(&["upload", "f", "--ip", "1.1.1.1", "-j", "2"]).is_err());
        assert!(parse(&["upload", "dir", "--restart"]).is_err());
        let Some(Commands::Upload {
            recursive,
            restart,
            name,
            ..
        }) = parse(&["upload", "dir", "-r", "--restart", "--name", "backup"])
            .unwrap()
            .command
        else {
            panic!()
        };
        assert!(recursive && restart);
        assert_eq!(name.as_deref(), Some("backup"));
        assert!(parse(&["upload", "f", "-r"]).is_ok());
        assert!(parse(&["mount", "--ip", "1.1.1.1"]).is_ok());
        assert!(parse(&["mount", "-j", "4"]).is_ok());
        let Some(Commands::MountHelper { endpoints, command }) = parse(&[
            "mount-helper",
            "--endpoint",
            "142.250.27.100",
            "--endpoint",
            "2404:6800:4003:c00::5f",
            "--",
            "rclone",
            "mount",
            "--read-only",
        ])
        .unwrap()
        .command
        else {
            panic!()
        };
        assert_eq!(endpoints.len(), 2);
        assert_eq!(command, ["rclone", "mount", "--read-only"]);
        assert!(parse(&["mount-helper", "--", "rclone"]).is_err());
        assert!(matches!(
            parse(&["logout"]).unwrap().command,
            Some(Commands::Logout {
                account: None,
                all: false
            })
        ));
        assert!(parse(&["logout", "a@example.com", "--all"]).is_err());
        assert!(parse(&["login", "--add"]).is_ok());
        let cli = parse(&["ls", "-a", "work"]).unwrap();
        assert_eq!(cli.account.as_deref(), Some("work"));
        assert!(parse(&["switch"]).is_err());
        assert!(
            parse(&["download", "id", "--google-only"])
                .unwrap()
                .google_only
        );
        assert!(parse(&["--google-only", "scan"]).unwrap().google_only);
        assert!(parse(&["download", "id", "--sni", "g.cn", "--no-sni"]).is_err());
        let cli = parse(&["download", "id", "--no-sni", "--ip", "1.1.1.1", "-o", "f"]).unwrap();
        let Some(Commands::Download { transfer, .. }) = cli.command else {
            panic!()
        };
        assert_eq!(transfer.route.sni().as_deref(), Some(""));
    }
    #[test]
    fn long_upload_paths_keep_their_file_name() {
        assert_eq!(tail("a/b.txt", 20), "a/b.txt");
        assert_eq!(tail("abcdefghij", 4), "…hij");
        assert_eq!(tail("你好世界", 5), "…世界");
        assert_eq!(tail("", 4), "");
    }
}
