extern crate jack;
#[macro_use]
extern crate slog;
extern crate slog_term;
extern crate slog_async;

#[macro_use]
extern crate clap;

#[macro_use]
extern crate serde_derive;

extern crate jam;

use std::io;

use slog::Drain;

use clap::{Arg, App, SubCommand};

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
    let VERSION = env!("CARGO_PKG_VERSION");
    let cargs = clap_app!(Jamyx =>
        (version: VERSION)
        (author: "Javier A. Pollak")
        (about: "Jackaudio mixer/patchbay suite written in rust")
        (@arg config: -c --config +takes_value "Sets custom config file path")
        (@arg verbosity: -v ... "Sets custom verbosity level")
    ).get_matches();


    // Logger setup

    let log_lvl = match cargs.occurrences_of("verbosity") {
        // 0 => slog::Level::Warning,
        0 => slog::Level::Info,
        1 => slog::Level::Debug,
        2 => slog::Level::Trace,
        c => { if c > 2 { slog::Level::Trace } else { slog::Level::Warning } }
    };

    let log = setup_log(log_lvl);
    let log = log.new(o!("version" => "0.0.1"));
    info!(log, "Logger init");


    // Parse config
    let config = config::parse(cargs.value_of("config").unwrap_or("config.json"), log.new(o!()));

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
