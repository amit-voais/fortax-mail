//! `fortax-mail --bridge on|off|status` — the switch for hiSAI's local bridge.
//!
//! The bridge is off until someone says otherwise, and this is the plainest way to say it while the
//! Settings screen catches up. It only writes one row in the app's own settings table; the listener
//! itself starts with the app.

use std::path::PathBuf;

use fortax_mail_core::bridge;
use fortax_mail_core::config::Paths;

/// Returns the exit code when the run was a `--bridge` call, and nothing when the app should start.
pub fn run_if_requested() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--bridge") {
        return None;
    }
    let want = args.next().unwrap_or_else(|| "status".into());
    Some(match apply(&want) {
        Ok(text) => {
            println!("{text}");
            0
        }
        Err(text) => {
            eprintln!("{text}");
            1
        }
    })
}

fn db_path() -> Result<PathBuf, String> {
    let paths = Paths::default_dirs().map_err(|e| format!("Fortax Mail has no data directory: {e}"))?;
    Ok(paths.db_file())
}

fn apply(want: &str) -> Result<String, String> {
    let path = db_path()?;
    if !path.exists() {
        return Err(format!(
            "Fortax Mail has not been opened on this account yet ({} is missing). Start the app once, then run this again.",
            path.display()
        ));
    }
    match want {
        "on" | "off" => {
            bridge::set_enabled(&path, want == "on").map_err(|e| e.to_string())?;
            if want == "off" {
                if let Ok(paths) = Paths::default_dirs() {
                    bridge::clear_state(&paths.data_dir);
                }
                Ok("The hiSAI bridge is off. Restart Fortax Mail to stop the listener.".into())
            } else {
                Ok("The hiSAI bridge is on. Restart Fortax Mail; it then listens on 127.0.0.1 and writes the token \
                    hiSAI reads into bridge.json in the application data directory."
                    .into())
            }
        }
        "status" => {
            let on = bridge::is_enabled(&path).map_err(|e| e.to_string())?;
            let paths = Paths::default_dirs().map_err(|e| format!("{e}"))?;
            let state = bridge::state_path(&paths.data_dir);
            let running = state.exists();
            Ok(format!(
                "hiSAI bridge: {}\nlistener file: {} ({})",
                if on { "on" } else { "off" },
                state.display(),
                if running { "present" } else { "not written" }
            ))
        }
        other => Err(format!(
            "Say `--bridge on`, `--bridge off` or `--bridge status`; `{other}` is none of those."
        )),
    }
}
