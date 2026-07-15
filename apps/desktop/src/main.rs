#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod activation;
mod app;
mod autostart;
mod branding;
mod connectivity_platform;
mod desktop_shell;
mod i18n;
mod integrations;
mod manual_lan_route;
mod model_config;
mod paths;
mod readiness;
mod services;
mod updater;
mod worker;

use anyhow::Result;
use eframe::egui;

use crate::{
    activation::{InstanceDisposition, LaunchMode, prepare_instance},
    app::AirWikiApp,
    paths::AppPaths,
};

fn main() -> Result<()> {
    let launch_mode = LaunchMode::from_args(std::env::args_os())?;
    let paths = AppPaths::discover()?;
    let instance = match prepare_instance(&paths, launch_mode)? {
        InstanceDisposition::Primary(instance) => instance,
        InstanceDisposition::Secondary => return Ok(()),
    };
    let _logging_guard = init_logging(&paths)?;
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting AirWiki");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("AirWiki")
            .with_icon(branding::window_icon())
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([880.0, 600.0])
            .with_visible(launch_mode == LaunchMode::Foreground),
        ..Default::default()
    };
    eframe::run_native(
        "AirWiki",
        options,
        Box::new(move |creation_context| {
            let app = AirWikiApp::new(creation_context, paths, launch_mode, instance)?;
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))
}

fn init_logging(paths: &AppPaths) -> Result<tracing_appender::non_blocking::WorkerGuard> {
    std::fs::create_dir_all(&paths.logs)?;
    let file = tracing_appender::rolling::daily(&paths.logs, "airwiki.log");
    let (writer, guard) = tracing_appender::non_blocking(file);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "airwiki=info,airwiki_=info,warn".into()),
        )
        .with_writer(writer)
        .with_ansi(false)
        .with_target(true)
        .try_init()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(guard)
}
