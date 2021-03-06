extern crate jack;
#[macro_use]
extern crate slog;
extern crate slog_async;
extern crate slog_term;

#[macro_use]
extern crate clap;

#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate serde_json;

extern crate jam;

use std::io;
use std::io::BufRead;
use std::sync::{Arc, RwLock};

use slog::Drain;

mod config;
mod jacon;
mod jamyxer;
mod server;
mod utils;

fn setup_log(verbosity: slog::Level) -> slog::Logger {
    let decorator = slog_term::TermDecorator::new().stderr().build();
    let drain = slog_term::CompactFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    let drain = slog::LevelFilter(drain, verbosity).fuse();

    slog::Logger::root(drain, o!())
}

fn main() {
    static VERSION: &str = env!("CARGO_PKG_VERSION");
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
        c => {
            if c > 2 {
                slog::Level::Trace
            } else {
                slog::Level::Warning
            }
        }
    };
    println!("verbosity count: {:?}", log_lvl);

    let log = setup_log(log_lvl);
    let log = log.new(o!("version" => "0.0.1"));
    info!(log, "Logger init");

    // Parse config
    let config = Arc::new(RwLock::new(config::parse(
        cargs.value_of("config").unwrap_or("config.json"),
        log.new(o!()),
    )));

    // Init JClient
    let mut jclient = jam::Client::new("Jacon", log.new(o!()));
    jclient.init(None).unwrap();

    // Set callbacks
    let cblog = log.new(o!());
    jclient.hook(jam::CB::client_registration(Box::new(move |_, cn, of| {
        info!(
            cblog,
            "Client {}registered: `{}`",
            if of { "" } else { "un" },
            cn
        );
    })));

    // setup jacon
    let mut jacon = jacon::ConnectionKit::new(
        log.clone(), jclient.jclient.clone(), config.clone());

    jacon.init(&mut jclient).expect("initializing jacon");

    // setup jamyxer
    let mut jamyxer = jamyxer::Patchbay::new(
        log.clone(), jclient.jclient.clone(), config.clone());

    jamyxer.init(&mut jclient);

    // =========== START ===========
    // Activate JClient
    // This activates the client and activates all the callbacks that were set
    debug!(log, "Starting jclient...");
    jclient.activate().unwrap();
    jclient.start_reconnection_loop().unwrap();

    debug!(log, "Starting Jacon...");
    jacon.start().expect("starting jacon");

    debug!(log, "Starting Jamyxer...");
    jamyxer.start();

    let sender = server::CmdSender::new(
        jamyxer.get_cmd_sender().expect("getting jamyxer cmd sender").clone(),
        jacon.  get_cmd_sender().expect("getting jacon cmd sender").clone(),
        jamyxer.get_cmd_sender().expect("getting jamyxer cmd sender").clone(),
    );
    debug!(log, "Starting server...");
    server::start(log.clone(), sender);

    debug!(log, "Done Activation phase!");
    /*
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        match line {
            Ok(line) => info!(log, "stdin receive: {:?}", line), // Do nothing
            Err(err) => println!("stdin read err: {:?}", err),
        }
    }
    */

    let mut user_input = String::new();
    io::stdin().read_line(&mut user_input).expect("waiting for user input");

    // Deactivate JClient
    jclient.deactivate().unwrap();
    info!(log, "Jclient deactivate");
}
