extern crate jack;
#[macro_use]
extern crate slog;
extern crate slog_term;
extern crate slog_async;
// extern crate sloggers;

extern crate serde;
extern crate serde_json;
#[macro_use]
extern crate serde_derive;

extern crate jam;

use std::io;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::channel;
use std::error::Error;
use std::thread;
use std::collections::HashMap;

// use serde_json as serde;
use jack::prelude as j;
use jam::JackClientUtils;
// use sloggers::Build;
// use sloggers::terminal::{TerminalLoggerBuilder, Destination};
// use sloggers::types::{Severity, Format};

use slog::Drain;


trait LogError {
    fn log_err(self, &slog::Logger) -> Self;
    fn warn_err(self, &slog::Logger) -> Self;
}

impl<T, E: std::fmt::Debug> LogError for Result<T, E> {
    fn log_err(self, logger: &slog::Logger) -> Self {
        return self.map_err(|expl| { error!(logger, "{:?}", expl); expl });
    }
    fn warn_err(self, logger: &slog::Logger) -> Self {
        return self.map_err(|expl| { warn!(logger, "{:?}", expl); expl });
    }
}

// trait LogOption<T> {
//     fn log_expect(self, &str) -> T;
// }
// impl<T> LogOption<T> for Option<T> {
//     fn log_expect(self, &str) -> T {

//     }
// }

#[derive(Serialize, Deserialize)]
struct Config {
    connections: HashMap<String, Vec<String>>
}

fn main() {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::CompactFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    let drain = slog::LevelFilter(drain, slog::Level::Debug).fuse();

    let log = slog::Logger::root(drain, o!("version" => "0.0.1"));

    info!(log, "Logger init");


    // Parse config
    let config = get_config(log.new(o!()));


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

    setup_jacon(log.new(o!()), &mut jclient, &config);

    // Activate JClient
    jclient.activate().unwrap();

    let mut user_input = String::new();
    io::stdin().read_line(&mut user_input).ok();

    // Deactivate JClient
    jclient.deactivate().unwrap();
    info!(log, "Jclient deactivate");
}

fn get_config(_: slog::Logger) -> Config {
    let path = std::path::Path::new("config.json");

    let mut file = match std::fs::File::open(&path) {
        Err(why) => panic!("couldn't open {}: {}", path.display(), why.description()),
        Ok(file) => file,
    };

    let mut s = String::new();
    if let Err(why) = file.read_to_string(&mut s) {
        panic!("couldn't read {}: {}", path.display(), why.description());
    }

    let config: Config = serde_json::from_str(&s).unwrap();

    return config;
}

fn setup_jacon(log: slog::Logger, jclient: &mut jam::Client, config: &Config) {
    let cons = config.connections.clone();
    let cli = jclient.jclient.clone(); // clones the Arc
    let logger = log.new(o!());

    jclient.hook(jam::CB::ports_connected(
        Box::new(move |c, i, o, connected| {
            let iname = c.port_name_by_id(i).unwrap(); // in
            let oname = c.port_name_by_id(o).unwrap(); // out

            let mut is_fine = false;
            for (ii, oos) in &cons {
                if &iname == ii && oos.contains(&oname) {
                    is_fine = connected;
                    break;
                }
            }

            let logger = logger.new(o!("stat" => if is_fine {"GOOD"} else {"BAD"}));
            info!(logger, "Ports {}connected: `{}` and `{}`", if connected {""} else {"dis"}, iname, oname);
            debug!(logger, "{}: ", if is_fine {"GOOD"} else {"BAD"});
            if !is_fine {
                let connecting = !connected;
                let cli = cli.clone();
                let logger = logger.new(o!());
                thread::spawn(move || {
                    let cli = cli.lock().unwrap();
                    let cli = cli.as_inactive().unwrap();
                    debug!(logger, "{}connecting ports `{}` and `{}`", if connecting {""} else {"dis"}, iname, oname);
                    cli.connect_ports_by_name_if(&connecting, &iname, &oname).log_err(&logger);
                });
            } else {
                debug!(logger, "Doing nothing");
            }
        })
    ));
}
