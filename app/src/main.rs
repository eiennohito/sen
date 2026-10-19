use line_wasm_poc::{
    backend::{FakeBackend, LineBackend, ProfileAction},
    controller::{ActionQueue, AppState, DetailPage, Panel},
    engine::{Connection, Delta, Intent, Sender, StoreLocation, Worker},
    store::*,
};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{cell::RefCell, collections::HashMap, rc::Rc};
mod account;
mod callbacks;
mod desktop;
mod reading;
mod runtime;
mod startup;
mod window;
use desktop::Desktop;
use runtime::Runtime;
#[cfg(test)]
use window::create_app_window;
mod assets;
mod attention;
mod attention_policy;
mod autostart;
mod background;
mod debug_launch;
mod demo;
mod demo_data;
mod desktop_integration;
mod event_loop;
#[cfg(feature = "ui-debug")]
mod latency;
mod links;
mod notifications;
mod present;
mod presenter;
mod preview;
mod rich_content;
mod snapshot;
mod text;
mod tray;
mod viewer;
pub use line_desktop_ui::*;
use present::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut demo = false;
    let mut snapshot_path = None;
    let mut size = (980, 720);
    let mut text_size = 15;
    let mut scene = String::new();
    let mut case = None;
    let mut dark = false;
    let mut snapshot_profile = false;
    let mut background = false;
    let mut locations = line_wasm_poc::locations::Locations::from_environment()?;
    let mut explicit_paths = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--build-info" => {
                println!(
                    "{{\"mcp_debug\":{},\"renderer\":\"software\"}}",
                    cfg!(feature = "ui-debug")
                );
                return Ok(());
            }
            "--demo" => demo = true,
            "--demo-paging" => {
                demo = true;
                scene = "paging".into();
            }
            "--demo-previews" => {
                demo = true;
                scene = "previews".into();
            }
            "--demo-dark" => {
                demo = true;
                dark = true;
            }
            "--demo-light" => demo = true,
            "--snapshot-profile" => snapshot_profile = true,
            "--snapshot-case" => case = Some(args.next().ok_or("missing snapshot case")?),
            "--debug-background" => background = true,
            "--snapshot" => {
                snapshot_path = Some(args.next().ok_or("missing snapshot path")?);
                demo = true;
            }
            "--snapshot-size" | "--demo-size" => {
                let v = args.next().ok_or("missing size")?;
                let (w, h) = v.split_once('x').ok_or("expected WIDTHxHEIGHT")?;
                size = (w.parse()?, h.parse()?);
            }
            "--snapshot-text-size" => {
                text_size = args.next().ok_or("missing text size")?.parse()?
            }
            "--snapshot-scene" => scene = args.next().ok_or("missing scene")?,
            "--state" => {
                locations.session = args.next().ok_or("missing state")?.into();
                explicit_paths = true;
            }
            "--wasm" => {
                locations.wasm = args.next().ok_or("missing wasm")?.into();
                explicit_paths = true;
            }
            "--cache-dir" => locations.cache = args.next().ok_or("missing cache directory")?.into(),
            "--demo-signed-out" | "--demo-rich" | "--demo-photos" | "--demo-groups"
            | "--demo-friends" => {
                demo = true;
                scene = arg.trim_start_matches("--demo-").into();
            }
            _ => return Err(format!("unknown option {arg}").into()),
        }
    }
    if !demo && !explicit_paths {
        locations.load_existing()?;
    }
    startup::start(startup::Options {
        demo,
        snapshot_path,
        size,
        text_size,
        scene,
        case,
        dark,
        snapshot_profile,
        background,
        locations,
    })
}

#[cfg(test)]
mod tests;
