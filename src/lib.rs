// Keep the application implementation in one compilation unit while exposing
// it to the thin desktop and mobile platform launchers.
pub use fortax_mail_core;
include!("main.rs");
