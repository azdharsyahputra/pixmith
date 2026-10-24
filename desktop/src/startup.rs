//! Startup safety: picking a renderer the machine supports, a log file, and errors the user can see.
//!
//! The app is a GUI program (no console on Windows), so a failure before the window appears used to look like
//! "nothing happens". Older GPUs (for example Intel HD 5500 on 5th-gen Core) often lack the DirectX 12 / Vulkan
//! support wgpu needs. So: probe wgpu first and fall back to OpenGL (glow), relaunch with OpenGL if wgpu still
//! fails while starting, log every step to `<app data>/pixmith.log`, and show an error dialog on failure or panic.

use crate::ui::gamedev::paths;
use std::io::Write;
use std::path::PathBuf;

/// Set by the parent when it relaunches the app with another renderer, so the child doesn't loop.
pub const RENDERER_ENV: &str = "PIXMITH_RENDERER";
const RELAUNCHED_ENV: &str = "PIXMITH_RENDERER_FALLBACK";

/// `<app data>/pixmith.log`.
pub fn log_path() -> PathBuf {
    paths::app_dir().join("pixmith.log")
}

/// Appends a line to the log (best effort; never fails the app).
pub fn log(line: &str) {
    let path = log_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Keep the log small: start over past 1 MiB.
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1 << 20) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let _ = writeln!(f, "[{secs}] {line}");
    }
}

/// Shows a blocking error dialog (also works before any window exists). `PIXMITH_NO_DIALOGS` (CI) only logs.
pub fn show_error(title: &str, text: &str) {
    if std::env::var_os("PIXMITH_NO_DIALOGS").is_some() {
        log(&format!("{title}: {text}"));
        return;
    }
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(title)
        .set_description(text)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// Logs panics and tells the user where the log is, instead of vanishing silently.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = format!("Pixmith crashed: {info}");
        log(&msg);
        show_error(
            "Pixmith crashed",
            &format!(
                "{info}\n\nDetails were written to:\n{}\n\nPlease include this file when you report the problem.",
                log_path().display()
            ),
        );
        default(info);
    }));
}

/// Which renderer to use and why.
pub struct Choice {
    pub renderer: eframe::Renderer,
    pub reason: String,
    /// True when this process was started by a failed attempt with the other renderer.
    pub is_fallback: bool,
}

/// Picks wgpu when it finds a GPU adapter, otherwise OpenGL. `PIXMITH_RENDERER=glow|wgpu` forces one.
pub fn choose_renderer() -> Choice {
    let is_fallback = std::env::var_os(RELAUNCHED_ENV).is_some();
    match std::env::var(RENDERER_ENV).map(|v| v.to_ascii_lowercase()).as_deref() {
        Ok("glow" | "gl" | "opengl") => {
            return Choice { renderer: eframe::Renderer::Glow, reason: "PIXMITH_RENDERER=glow".into(), is_fallback };
        }
        Ok("wgpu") => {
            return Choice { renderer: eframe::Renderer::Wgpu, reason: "PIXMITH_RENDERER=wgpu".into(), is_fallback };
        }
        _ => {}
    }
    match probe_wgpu() {
        Ok(adapter) => {
            Choice { renderer: eframe::Renderer::Wgpu, reason: format!("wgpu adapter: {adapter}"), is_fallback }
        }
        Err(e) => Choice {
            renderer: eframe::Renderer::Glow,
            reason: format!("no usable wgpu adapter ({e}); using OpenGL"),
            is_fallback,
        },
    }
}

/// Asks wgpu for an adapter on the backends eframe uses, without creating a window.
fn probe_wgpu() -> Result<String, String> {
    use eframe::wgpu;
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    desc.backends = wgpu::Backends::PRIMARY;
    let instance = wgpu::Instance::new(desc);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        .map_err(|e| e.to_string())?;
    let info = adapter.get_info();
    // A software rasterizer (no real GPU) works but is slow; OpenGL through the real driver is better.
    if info.device_type == wgpu::DeviceType::Cpu && cfg!(windows) {
        return Err(format!("only a software adapter ({})", info.name));
    }
    Ok(format!("{} ({:?}, {:?}, driver {})", info.name, info.backend, info.device_type, info.driver_info))
}

/// After wgpu failed to start: run this program again with OpenGL. Returns true when the child was started.
pub fn relaunch_with_opengl() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    std::process::Command::new(exe).args(args).env(RENDERER_ENV, "glow").env(RELAUNCHED_ENV, "1").spawn().is_ok()
}

/// Writes a short report about the machine and the renderer choice, for `Pixmith --diagnose`.
pub fn diagnose() -> String {
    let choice = choose_renderer();
    let report = format!(
        "Pixmith {} on {} {}\nrenderer: {:?}\nreason: {}\nlog: {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        choice.renderer,
        choice.reason,
        log_path().display()
    );
    log(&format!("diagnose:\n{report}"));
    report
}
