//! Material 3 AI — a Material 3 desktop app for Claude Code. It drives the
//! user's own `claude` CLI through its stream-json protocol.

mod app;
mod chat;
mod claude;
mod config;
mod images;
mod sys;
mod theme;
mod ui;

use std::path::PathBuf;

const USAGE: &str = "Usage: m3-ai [OPTIONS] [DIR]

Opens a new chat with Claude Code in DIR (a project folder).

Options:
  --resume <ID>  Reopen a chat by its session id
  -h, --help     Show this help
  -V, --version";

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let mut dir = None;
    let mut resume = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            "-V" | "--version" => {
                println!("m3-ai {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--resume" | "-r" => match args.next() {
                Some(id) => resume = Some(id),
                None => {
                    eprintln!("--resume needs a session id\n\n{USAGE}");
                    std::process::exit(2);
                }
            },
            other if other.starts_with('-') => {
                eprintln!("Unknown option: {other}\n\n{USAGE}");
                std::process::exit(2);
            }
            path => {
                let p = PathBuf::from(path);
                dir = Some(std::fs::canonicalize(&p).unwrap_or(p));
            }
        }
    }

    // Development: `M3_AI_WINDOW=1240x1400` sets the starting size.
    let size = std::env::var("M3_AI_WINDOW")
        .ok()
        .and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some([w.parse().ok()?, h.parse().ok()?])
        })
        .unwrap_or([1240.0, 840.0]);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Material 3 AI")
            .with_app_id("material_3_ai")
            .with_inner_size(size)
            .with_min_inner_size([760.0, 520.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "material_3_ai",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, app::Startup { dir, resume })))),
    )
}
