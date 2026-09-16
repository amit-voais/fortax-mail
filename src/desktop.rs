#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(not(target_os = "ios"))]
    if let Some(code) = fortax_mail::pdf_preview::run_worker_if_requested() {
        std::process::exit(code);
    }
    // `--bridge on|off|status` answers and exits; the app itself starts only without it.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    if let Some(code) = fortax_mail::bridge_cli::run_if_requested() {
        std::process::exit(code);
    }
    fortax_mail::run_desktop(fortax_mail::PlatformContext::desktop()?)
}
