use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bluelink_ui::App;
use eframe::egui;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

fn main() -> eframe::Result<()> {
    let vault_arg = parse_vault_arg(std::env::args().skip(1));
    let reload_flag = Arc::new(AtomicBool::new(false));

    let reload_for_ui = reload_flag.clone();
    let vault_for_ui = vault_arg.clone();

    eframe::run_native(
        "BlueLink",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1280.0, 840.0])
                .with_min_inner_size([900.0, 600.0])
                .with_title("BlueLink"),
            ..Default::default()
        },
        Box::new(move |cc| {
            let pick: bluelink_ui::FolderPicker = Box::new(|| {
                rfd::FileDialog::new()
                    .set_title("Open BlueLink vault")
                    .pick_folder()
            });
            let app = App::new(cc, vault_for_ui.clone(), reload_for_ui, Some(pick));
            let mut desktop = DesktopApp {
                inner: app,
                watcher: None,
                reload_flag,
                watching: None,
            };
            if let Some(path) = vault_for_ui {
                desktop.attach_watcher(&path);
            }
            Ok(Box::new(desktop) as Box<dyn eframe::App>)
        }),
    )
}

struct DesktopApp {
    inner: App,
    watcher: Option<RecommendedWatcher>,
    reload_flag: Arc<AtomicBool>,
    watching: Option<PathBuf>,
}

impl eframe::App for DesktopApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if let Some(root) = self.inner.vault_root() {
            let root = root.to_path_buf();
            let needs = self
                .watching
                .as_ref()
                .map(|w| w != &root)
                .unwrap_or(true);
            if needs {
                self.attach_watcher(&root);
            }
        }

        if self.reload_flag.load(Ordering::Relaxed) {
            // Coalesce notify bursts — don't spin the event loop at max rate.
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        self.inner.update(ctx, frame);
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
        // Ignore meta dir and atomic-write temps (`.name.tmp-PID`).
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

fn parse_vault_arg(mut args: impl Iterator<Item = String>) -> Option<PathBuf> {
    while let Some(a) = args.next() {
        if a == "--vault" {
            return args.next().map(PathBuf::from);
        }
        if let Some(path) = a.strip_prefix("--vault=") {
            return Some(PathBuf::from(path));
        }
    }
    None
}
