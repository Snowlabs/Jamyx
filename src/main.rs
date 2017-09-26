extern crate jack;
#[macro_use]
extern crate slog;
extern crate slog_term;
extern crate slog_async;

// extern crate serde;
// extern crate serde_json;
#[macro_use]
extern crate serde_derive;

extern crate jam;

use std::io;

use slog::Drain;

mod config;
mod jacon;
mod utils;

fn setup_log(verbosity: slog::Level) -> slog::Logger {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::CompactFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    let drain = slog::LevelFilter(drain, verbosity).fuse();

    slog::Logger::root(drain, o!())
}

fn main() {
    // Logger setup
    let log = setup_log(slog::Level::Debug);
    let log = log.new(o!("version" => "0.0.1"));
    info!(log, "Logger init");


    // Parse config
    let config = config::parse("config.json", log.new(o!()));

    // Init JClient
    let mut jclient = jam::Client::new("Jacon", log.new(o!()));
    jclient.init(None).unwrap();

    // Set callbacks
    let cblog = log.new(o!());
    jclient.hook(jam::CB::client_registration(
        Box::new(move |_, cn, of| {
            info!(cblog, "Client {}registered: `{}`", if of {""} else {"un"}, cn);
        })
    ));

    // setup jacon
    let jacon_sender = jacon::setup(log.new(o!()), &mut jclient, &config);

    // =========== START ===========
    // Activate JClient
    // This activates the client and activates all the callbacks that were set
    jclient.activate().unwrap();
    jclient.start_reconnection_loop().unwrap();

    jacon::start(jacon_sender);

    let mut user_input = String::new();
    io::stdin().read_line(&mut user_input).ok();

    // Deactivate JClient
    jclient.deactivate().unwrap();
    info!(log, "Jclient deactivate");
}
