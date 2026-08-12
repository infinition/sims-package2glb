// Windows: no console window behind the application.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    sims_package2glb_lib::run()
}
