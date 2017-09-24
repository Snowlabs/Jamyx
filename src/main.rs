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
use std::sync::mpsc::{channel, Sender};
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

enum JaconSignals {
    CheckConection(String, String, bool),
    SetConnectionCheck(bool),
    DisconnectAll,
    ReconnectGood,
    RetryConnection(String, String, bool, u64),
}

macro_rules! as_inactive {
    ($cli:ident) => {
        let $cli = $cli.lock().unwrap();
        let $cli = $cli.as_inactive().unwrap();
    }
}

trait RetryConnection {
    fn retry_if_fail_in(&self, bool, String, String, &Sender<JaconSignals>, u64);
}

impl RetryConnection for Result<(), j::JackErr> {
    fn retry_if_fail_in(&self, of: bool, a: String, b: String, tSig: &Sender<JaconSignals>, delay: u64) {
        if let &Err(ref e) = self { match e {
                &j::JackErr::PortConnectionError(_,_) | &j::JackErr::PortDisconnectionError => {
                    tSig.send(JaconSignals::RetryConnection(a, b, of, delay));
                }, _ => ()
        } }
    }
}


fn setup_jacon(log: slog::Logger, jclient: &mut jam::Client, config: &Config) {

    let cons = config.connections.clone();
    let cli = jclient.jclient.clone(); // clones the Arc
    let logger = log.new(o!());
    let (_tSig, rSig) = channel();

    let tSig = _tSig.clone();
    thread::spawn(move || {
        let mut disable_check_connections = false;
        loop {
            let sig = rSig.recv().unwrap();
            match sig {
                JaconSignals::SetConnectionCheck(of) => { disable_check_connections = !of },
                JaconSignals::RetryConnection(iname, oname, connecting, delay) => {
                    warn!(logger, "{}connection Failed and got rescheduled: `{}` and `{}`",
                           if connecting {""} else {"dis"}, iname, oname);
                    debug!(logger, "retrying in {} milliseconds...", delay);

                    let cli = cli.clone();
                    let logger = logger.clone();
                    let tSig = tSig.clone();
                    thread::spawn(move || {
                        thread::sleep(std::time::Duration::from_millis(delay));
                        as_inactive!(cli);
                        cli
                            .connect_ports_by_name_if(&connecting, &iname, &oname)
                            .log_err(&logger)
                            .map_err(|e| { warn!(logger, "FAILED again! Rescheduling!"); e })
                            .retry_if_fail_in(connecting, iname, oname, &tSig, 100);
                    });
                },
                JaconSignals::CheckConection(iname, oname, connected) => {
                    if disable_check_connections {
                        debug!(logger, "Skipping connection checks");
                    } else {
                        let mut is_fine = !connected;
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
                            as_inactive!(cli);
                            debug!(logger, "{}connecting ports `{}` and `{}`", if connecting {""} else {"dis"}, iname, oname);
                            cli
                                .connect_ports_by_name_if(&connecting, &iname, &oname)
                                .log_err(&logger)
                                .map_err(|e| { warn!(logger, "FAILED! Rescheduling!"); e })
                                .retry_if_fail_in(connecting, iname, oname, &tSig, 100);

                        } else {
                            debug!(logger, "Doing nothing");
                        }
                    }
                }
                JaconSignals::ReconnectGood => {
                    as_inactive!(cli);
                    for (ii, oos) in &cons { for oo in oos {
                            cli.connect_ports_by_name(&ii, &oo);
                    } }
                }
                JaconSignals::DisconnectAll => {
                    tSig.send(JaconSignals::SetConnectionCheck(false));
                    as_inactive!(cli);
                    for ii in cli.ports(None, None, j::port_flags::IS_INPUT) {
                        cli.port_by_name(&ii).unwrap().disconnect();
                    }
                    tSig.send(JaconSignals::SetConnectionCheck(true));
                }
            }
        }
    });

    let tSig = _tSig.clone();
    jclient.hook(jam::CB::ports_connected(
        Box::new(move |c, i, o, connected| {
            let iname = c.port_name_by_id(i).unwrap(); // in
            let oname = c.port_name_by_id(o).unwrap(); // out

            tSig.send(JaconSignals::CheckConection(iname, oname, connected));
        })
    ));
    let tSig = _tSig.clone();
    tSig.send(JaconSignals::DisconnectAll);
    tSig.send(JaconSignals::ReconnectGood);
}
