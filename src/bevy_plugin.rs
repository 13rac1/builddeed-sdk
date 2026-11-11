//! A Bevy plugin (feature `bevy`).
//!
//! ```no_run
//! use bevy::prelude::*;
//! use builddeed::bevy_plugin::{BuildDeedPlugin, License, RegisterRequest};
//! use builddeed::Product;
//!
//! fn main() {
//!     App::new()
//!         .add_plugins(MinimalPlugins)
//!         .add_plugins(BuildDeedPlugin::new(Product::testnet(
//!             "doom.builddeed.testnet",
//!             "doom",
//!             env!("CARGO_PKG_VERSION"),
//!         )))
//!         .add_systems(Update, show)
//!         .run();
//! }
//!
//! fn show(license: Res<License>, keys: Res<ButtonInput<KeyCode>>, mut register: EventWriter<RegisterRequest>) {
//!     if !license.is_registered() && keys.just_pressed(KeyCode::KeyR) {
//!         register.send_default();
//!     }
//! }
//! ```
//!
//! At start the plugin asks the contract for this computer's seat. A
//! [`RegisterRequest`] opens the register page and waits for the seat in a
//! worker thread; a second request while waiting stops the wait. A
//! [`BuyRequest`] opens the buy page.

use crate::{Client, Error, Product, Seat};
use bevy::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How often the plugin asks for the seat while waiting, and for how long.
const POLL_EVERY: Duration = Duration::from_secs(5);
const REGISTER_WAIT: Duration = Duration::from_secs(600);

/// Adds the [`License`] resource and the request events.
pub struct BuildDeedPlugin {
    product: Product,
}

impl BuildDeedPlugin {
    /// The plugin for `product`.
    pub fn new(product: Product) -> Self {
        Self { product }
    }
}

impl Plugin for BuildDeedPlugin {
    fn build(&self, app: &mut App) {
        let (tx, rx) = channel();
        app.insert_resource(License {
            state: LicenseState::Checking,
            client: Client::new(self.product.clone()),
            message: String::new(),
            stop: None,
        })
        .insert_resource(Inbox {
            tx,
            rx: Mutex::new(rx),
        })
        .add_event::<RegisterRequest>()
        .add_event::<BuyRequest>()
        .add_systems(Startup, start_check)
        .add_systems(Update, (receive, on_register, on_buy));
    }
}

/// The license state of this computer.
#[derive(Debug, Clone, PartialEq)]
pub enum LicenseState {
    /// The first check has not answered yet.
    Checking,
    /// The computer has a seat on a license that covers this version.
    Registered(Seat),
    /// No seat, or the check failed (see [`License::message`]).
    Unregistered,
    /// The register page is open; the plugin waits for the seat.
    Waiting,
}

/// The license of this computer.
#[derive(Resource, Debug)]
pub struct License {
    /// The current state.
    pub state: LicenseState,
    /// The client, for the computer id and URLs.
    pub client: Client,
    /// A line for the player, for example an error. Empty when there is none.
    pub message: String,
    stop: Option<Arc<AtomicBool>>,
}

impl License {
    /// True when the computer has a seat that covers this version.
    pub fn is_registered(&self) -> bool {
        matches!(self.state, LicenseState::Registered(_))
    }
}

/// Send it to register this computer, or to stop waiting.
#[derive(Event, Debug, Default)]
pub struct RegisterRequest;

/// Send it to open the buy page.
#[derive(Event, Debug, Default)]
pub struct BuyRequest;

enum Msg {
    Check(Result<Option<Seat>, Error>),
    Registered(Result<Seat, Error>),
}

#[derive(Resource)]
struct Inbox {
    tx: Sender<Msg>,
    rx: Mutex<Receiver<Msg>>,
}

fn start_check(license: Res<License>, inbox: Res<Inbox>) {
    let client = license.client.clone();
    let tx = inbox.tx.clone();
    std::thread::spawn(move || {
        let _ = tx.send(Msg::Check(client.check()));
    });
}

fn receive(inbox: Res<Inbox>, mut license: ResMut<License>) {
    let Ok(rx) = inbox.rx.lock() else {
        return;
    };
    while let Ok(msg) = rx.try_recv() {
        apply(&mut license, msg);
    }
}

fn apply(license: &mut License, msg: Msg) {
    match msg {
        Msg::Check(Ok(Some(s))) => license.state = LicenseState::Registered(s),
        Msg::Check(Ok(None)) => license.state = LicenseState::Unregistered,
        Msg::Check(Err(e)) => {
            license.message = format!("Cannot check the license: {e}");
            license.state = LicenseState::Unregistered;
        }
        Msg::Registered(Ok(s)) => {
            license.stop = None;
            license.message.clear();
            license.state = LicenseState::Registered(s);
        }
        Msg::Registered(Err(e)) => {
            license.stop = None;
            license.message = e.to_string();
            license.state = LicenseState::Unregistered;
        }
    }
}

fn on_register(
    mut requests: EventReader<RegisterRequest>,
    mut license: ResMut<License>,
    inbox: Res<Inbox>,
) {
    if requests.read().count() == 0 {
        return;
    }
    if let Some(stop) = license.stop.take() {
        stop.store(true, Ordering::Relaxed);
        license.state = LicenseState::Unregistered;
        license.message.clear();
        return;
    }
    if license.is_registered() {
        return;
    }
    if let Err(e) = license.client.open_register_page() {
        license.message = e.to_string();
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    license.stop = Some(stop.clone());
    license.state = LicenseState::Waiting;
    license.message.clear();
    let client = license.client.clone();
    let tx = inbox.tx.clone();
    std::thread::spawn(move || {
        let result = client.wait_for_seat(&stop, POLL_EVERY, REGISTER_WAIT);
        if !stop.load(Ordering::Relaxed) {
            let _ = tx.send(Msg::Registered(result));
        }
    });
}

fn on_buy(mut requests: EventReader<BuyRequest>, mut license: ResMut<License>) {
    if requests.read().count() > 0 {
        if let Err(e) = license.client.open_buy_page() {
            license.message = e.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_rpc;

    #[test]
    fn startup_check_sets_the_state() {
        let url = fake_rpc::serve(vec![fake_rpc::seat("7", "c.testnet", true)]);
        let product = Product::testnet("x.testnet", "x", "1").with_rpc_url(url);
        let mut app = App::new();
        app.add_plugins(BuildDeedPlugin::new(product.clone()));
        // A fixed computer id, so that the test does not need /etc/machine-id.
        app.world_mut().resource_mut::<License>().client =
            Client::with_device(product, "a".repeat(64));
        app.update();
        for _ in 0..200 {
            if app.world().resource::<License>().is_registered() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
            app.update();
        }
        let license = app.world().resource::<License>();
        assert!(
            matches!(&license.state, LicenseState::Registered(s) if s.account_id == "c.testnet")
        );
    }

    #[test]
    fn errors_go_to_the_message() {
        let mut license = License {
            state: LicenseState::Checking,
            client: Client::with_device(Product::testnet("x", "x", "1"), "a".repeat(64)),
            message: String::new(),
            stop: None,
        };
        apply(&mut license, Msg::Check(Err(Error::Network("down".into()))));
        assert_eq!(license.state, LicenseState::Unregistered);
        assert!(license.message.contains("down"));
        apply(
            &mut license,
            Msg::Registered(Err(Error::LicenseEnded {
                token_id: "3".into(),
            })),
        );
        assert!(license.message.contains("#3"));
    }
}
