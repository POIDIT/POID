//! POID Studio entry point.
//!
//! The `windows_subsystem` attribute is UX rule 1 in binary form: a release
//! build is a GUI process from the first instruction, so a console window can
//! never appear, not even as a flash during startup. Debug builds keep the
//! console so `tauri dev` logging has somewhere to go.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    poid_studio_lib::run()
}
