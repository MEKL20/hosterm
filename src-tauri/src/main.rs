// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Windows SSH_ASKPASS helper entry: when sftp.exe launches this binary with
    // a password prompt as its only argument, answer it and exit before the
    // GUI is built.
    hosterm_lib::maybe_run_askpass();
    hosterm_lib::run()
}
