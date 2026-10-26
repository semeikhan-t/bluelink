mod sync_runtime;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bluelink_ui::{sync_ui, App};
use eframe::egui;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use sync_runtime::{SyncRuntime, TransportMode};

fn main() -> eframe::Result<()> {
    let args = CliArgs::parse(std::env::args().skip(1));
    if args.help {
        print_help();
        return Ok(());
    }
    if let Some(err) = args.error.as_ref() {
        eprintln!("bluelink: {err}");
        print_help();
        std::process::exit(2);
    }

    let reload_flag = Arc::new(AtomicBool::new(false));
    let initiator = matches!(args.mode, TransportMode::TcpInitiator(_));
    let (sync_handle, sync_io) = sync_ui::channel_with_role(initiator);
    let sync_runtime = SyncRuntime::start(sync_io, args.mode.clone());
    let sync_wake = sync_handle.wake_flag();

    let reload_for_ui = reload_flag.clone();
    let vault_for_ui = args.vault.clone();
    let title = if initiator {
        "BlueLink (Initiator)"
    } else if matches!(args.mode, TransportMode::TcpHost(_)) {
        "BlueLink (Host TCP)"
    } else {
        "BlueLink"
    };

    eframe::run_native(
        title,
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1280.0, 840.0])
                .with_min_inner_size([900.0, 600.0])
                .with_title(title),
            ..Default::default()
        },
        Box::new(move |cc| {
            let pick: bluelink_ui::FolderPicker = Box::new(|| {
                rfd::FileDialog::new()
                    .set_title("Open BlueLink vault")
                    .pick_folder()
            });
            let app = App::new(
                cc,
                vault_for_ui.clone(),
                reload_for_ui,
                Some(pick),
                Some(sync_handle),
            );
            let mut desktop = DesktopApp {
                inner: app,
                watcher: None,
                reload_flag,
                sync_wake,
                sync_runtime,
                watching: None,
            };
            if let Some(path) = vault_for_ui {
                desktop.attach_watcher(&path);
                desktop.sync_runtime.set_vault(Some(path));
            } else if let Some(root) = desktop.inner.vault_root() {
                let root = root.to_path_buf();
                desktop.attach_watcher(&root);
                desktop.sync_runtime.set_vault(Some(root));
            }
            Ok(Box::new(desktop) as Box<dyn eframe::App>)
        }),
    )
}

struct DesktopApp {
    inner: App,
    watcher: Option<RecommendedWatcher>,
    reload_flag: Arc<AtomicBool>,
    sync_wake: Arc<AtomicBool>,
    sync_runtime: SyncRuntime,
    watching: Option<PathBuf>,
}

impl eframe::App for DesktopApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let t0 = std::time::Instant::now();

        if let Some(root) = self.inner.vault_root() {
            let root = root.to_path_buf();
            let needs = self
                .watching
                .as_ref()
                .map(|w| w != &root)
                .unwrap_or(true);
            if needs {
                self.attach_watcher(&root);
                self.sync_runtime.set_vault(Some(root));
            }
        }

        let sync_woke = self.sync_wake.swap(false, Ordering::SeqCst);
        if self.reload_flag.load(Ordering::Relaxed) || sync_woke {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        self.inner.update(ctx, frame);

        let ms = t0.elapsed().as_millis();
        if ms > 100 {
            eprintln!("bluelink: slow frame {ms}ms");
        }
    }
}

impl DesktopApp {
    fn attach_watcher(&mut self, path: &Path) {
        match start_watcher(path, self.reload_flag.clone()) {
            Ok(w) => {
                self.watcher = Some(w);
                self.watching = Some(path.to_path_buf());
            }
            Err(e) => {
                eprintln!("file watcher unavailable: {e}");
                self.watcher = None;
                self.watching = Some(path.to_path_buf());
            }
        }
    }
}

fn start_watcher(
    path: &Path,
    flag: Arc<AtomicBool>,
) -> Result<RecommendedWatcher, notify::Error> {
    let mut watcher = notify::recommended_watcher(move |res: Result<notify::Event, _>| {
        let Ok(event) = res else {
            return;
        };
        if !matches!(
            event.kind,
            notify::EventKind::Modify(_)
                | notify::EventKind::Create(_)
                | notify::EventKind::Remove(_)
                | notify::EventKind::Any
        ) {
            return;
        }
        let relevant = event.paths.iter().any(|p| {
            let s = p.to_string_lossy();
            !s.contains("/.bluelink/") && !s.contains(".tmp-")
        });
        if relevant {
            flag.store(true, Ordering::SeqCst);
        }
    })?;
    watcher.watch(path, RecursiveMode::Recursive)?;
    Ok(watcher)
}

#[derive(Debug)]
struct CliArgs {
    vault: Option<PathBuf>,
    mode: TransportMode,
    help: bool,
    error: Option<String>,
}

impl CliArgs {
    fn parse(args: impl Iterator<Item = String>) -> Self {
        let mut vault = None;
        let mut sync_tcp: Option<SocketAddr> = None;
        let mut connect: Option<SocketAddr> = None;
        let mut initiator = false;
        let mut help = false;
        let mut error = None;

        let mut args = args.peekable();
        while let Some(a) = args.next() {
            match a.as_str() {
                "-h" | "--help" => help = true,
                "--vault" => {
                    if let Some(p) = args.next() {
                        vault = Some(PathBuf::from(p));
                    } else {
                        error = Some("--vault needs a path".into());
                    }
                }
                "--initiator" => initiator = true,
                "--sync-tcp" => match args.next().as_deref().map(parse_addr) {
                    Some(Ok(addr)) => sync_tcp = Some(addr),
                    Some(Err(e)) => error = Some(format!("--sync-tcp: {e}")),
                    None => error = Some("--sync-tcp needs ADDR (e.g. 127.0.0.1:7878)".into()),
                },
                "--connect" => match args.next().as_deref().map(parse_addr) {
                    Some(Ok(addr)) => connect = Some(addr),
                    Some(Err(e)) => error = Some(format!("--connect: {e}")),
                    None => error = Some("--connect needs ADDR (e.g. 127.0.0.1:7878)".into()),
                },
                other if other.starts_with("--vault=") => {
                    vault = Some(PathBuf::from(other.trim_start_matches("--vault=")));
                }
                other if other.starts_with('-') => {
                    error = Some(format!("unknown flag {other}"));
                }
                _ => {}
            }
        }

        let mode = if error.is_some() {
            TransportMode::Ble
        } else if initiator {
            match connect {
                Some(addr) => TransportMode::TcpInitiator(addr),
                None => {
                    error = Some("--initiator requires --connect ADDR".into());
                    TransportMode::Ble
                }
            }
        } else if let Some(addr) = sync_tcp {
            if connect.is_some() {
                error = Some("use either --sync-tcp (Host) or --initiator --connect".into());
            }
            TransportMode::TcpHost(addr)
        } else if connect.is_some() {
            error = Some("--connect requires --initiator".into());
            TransportMode::Ble
        } else {
            TransportMode::Ble
        };

        Self {
            vault,
            mode,
            help,
            error,
        }
    }
}

fn parse_addr(s: &str) -> Result<SocketAddr, String> {
    s.parse()
        .map_err(|_| format!("invalid socket address `{s}` (want host:port)"))
}

fn print_help() {
    eprintln!(
        "\
BlueLink desktop

Usage:
  bluelink [--vault PATH]                          BLE Host (default)
  bluelink --vault PATH --sync-tcp 127.0.0.1:7878  TCP Host (no BLE)
  bluelink --vault PATH --initiator --connect 127.0.0.1:7878

TCP demo (two terminals):
  1) Host:      bluelink --vault ~/BlueLinkHost --sync-tcp 127.0.0.1:7878
                Devices → Show code
  2) Initiator: bluelink --vault ~/BlueLinkPhone --initiator --connect 127.0.0.1:7878
                Devices → enter code → Join
                Sync → Synchronize
"
    );
}
