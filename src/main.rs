mod browser;
mod format;
mod fsops;
mod model;
mod openers;
mod theme;
mod version;

use std::path::PathBuf;

use gpui::{
    px, size, App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions,
};

fn main() {
    let mut args = std::env::args().skip(1);
    if let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            print_usage();
            return;
        }
        if arg == "--version" || arg == "-V" {
            println!("beefile {}", version::label());
            return;
        }
        let (cwd, select) = fsops::launch_target(Some(PathBuf::from(arg)));
        run(cwd, select);
        return;
    }
    let (cwd, select) = fsops::launch_target(None);
    run(cwd, select);
}

fn print_usage() {
    println!(
        "\
BeeFile {version}
A fast file manager for Omarchy.

Usage:
  beefile [PATH]
  beefile --help

PATH is a directory, or a file (opens its folder and selects the file).
With no PATH, BeeFile opens the current directory.

Press ? in the window for keys. Scripts:
  scripts/dev.sh              debug build and run
  scripts/run.sh              release build and run
  scripts/test.sh             unit tests
  scripts/smoke.sh            build, launch on Wayland, expect it to stay open
  scripts/install-omarchy.sh  install the binary and enable the bar plugin",
        version = version::label()
    );
}

fn run(cwd: PathBuf, select: Option<String>) {
    Application::new().run(move |cx: &mut App| {
        browser::bind_keys(cx);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(1120.), px(740.)), cx);
        let open = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(format!("BeeFile {}", version::label()).into()),
                    ..Default::default()
                }),
                app_id: Some("BeeFile".into()),
                window_min_size: Some(size(px(720.), px(420.))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| browser::Browser::new(cwd, select, window, cx)),
        );
        if let Err(err) = open {
            eprintln!("BeeFile failed to open a window: {err:#}");
            cx.quit();
            return;
        }
        cx.activate(true);
    });
}
