#[cfg(feature = "demo")]
use line::backend::FakeBackend;
mod action_queue;
use action_queue::ActionQueue;
use line::{
    backend::{LineBackend, ProfileAction},
    controller::{AppState, DetailPage, Panel},
    engine::{Connection, Delta, Intent, Sender, StoreLocation, Worker},
    store::*,
};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{cell::RefCell, collections::HashMap, rc::Rc};
mod callbacks;
mod desktop;
mod desktop_env;
#[cfg(feature = "demo")]
mod dev;
mod focus;
mod reading;
mod runtime;
#[cfg(any(test, feature = "demo"))]
mod snapshot;
mod startup;
mod window;
use desktop::Desktop;
use runtime::Runtime;
#[cfg(test)]
use window::create_app_window;
mod assets;
mod emoji;
mod event_loop;
mod export;
mod notifications;
mod presenter;
mod rich_content;
mod text;
mod viewer;
pub use line_desktop_ui::*;
use presenter::format::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "demo")]
    let mut dev_args = dev::demo::Args::default();
    let mut client_version = None;
    let mut wasm_override = None;
    let mut locations = line::locations::Locations::from_environment()?;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--build-info" => {
                println!(
                    "{{\"mcp_debug\":{},\"demo\":{},\"renderer\":\"software\"}}",
                    cfg!(feature = "ui-debug"),
                    cfg!(feature = "demo")
                );
                return Ok(());
            }
            "--state" => {
                locations.session = args.next().ok_or("missing state")?.into();
            }
            "--client-version" => {
                client_version = Some(args.next().ok_or("missing client version")?)
            }
            "--wasm" => {
                wasm_override = Some(args.next().ok_or("missing wasm")?.into());
            }
            "--cache-dir" => locations.cache = args.next().ok_or("missing cache directory")?.into(),
            #[cfg(feature = "demo")]
            _ if dev_args.parse(&arg, &mut args)? => {}
            _ => return Err(format!("unknown option {arg}").into()),
        }
    }
    startup::start(
        locations,
        client_version,
        wasm_override,
        #[cfg(feature = "demo")]
        dev_args,
    )
}

#[cfg(test)]
mod tests;
