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
    jclient.start_reconnection_loop();

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

#[derive(Debug)]
enum JaconSignals {
    CheckConection(String, String, bool),
    SetConnectionCheck(bool),
    DisconnectAll,
    ReconnectGood,

    RetryIn(u64, Box<JaconSignals>),
    TryConnection(bool, String, String),

    ReconnectPort(String),
}

macro_rules! as_inactive {
    ($cli:ident, $log:ident, $if:block) => {
        if let Ok($cli) = $cli.lock().log_err(&$log) {
            if let Ok($cli) = $cli.as_inactive().log_err(&$log) $if
            else { error!($log, "Cannot do anything :("); }
        } else { error!($log, "Cannot do anything :("); }
    }
}

trait RetryConnection {
    fn retry_if_fail_in(&self, u64, JaconSignals, &Sender<JaconSignals>);
}

impl RetryConnection for Result<(), j::JackErr> {
    fn retry_if_fail_in(&self, delay: u64, sig: JaconSignals, tSig: &Sender<JaconSignals>) {
        if let &Err(ref e) = self { match e {
                &j::JackErr::PortConnectionError(_,_) => {
                    tSig.send(JaconSignals::RetryIn(delay, Box::new(sig)));
                }
                &j::JackErr::PortDisconnectionError => {
                    // tSig.send(JaconSignals::RetryIn(delay, Box::new(sig)));
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
                JaconSignals::ReconnectPort(port_name) => {
                    info!(logger, "Reevaluating connections for port: `{}`",
                          port_name);
                    as_inactive!(cli, logger, {
                        let port = cli.port_by_name(&port_name).unwrap();
                        let is_input = port.flags().contains(j::port_flags::IS_INPUT);
                        port.disconnect().unwrap();
                        for (oo, iis) in &cons { for ii in iis {
                            if (is_input && ii == &port_name) || (!is_input && oo == &port_name) {
                                tSig.send(JaconSignals::TryConnection(true, oo.clone(), ii.clone()));
                            }
                        } }
                    });
                }
                JaconSignals::RetryIn(delay, sig) => {
                    warn!(logger, "{:?} failed and got rescheduled", *sig);
                    debug!(logger, "scheduling retry in {} milliseconds...", delay);
                    let tSig = tSig.clone();
                    thread::spawn(move || {
                        thread::sleep(std::time::Duration::from_millis(delay));
                        tSig.send(*sig);
                    });
                }
                JaconSignals::TryConnection(of, oname, iname) => {
                    debug!(logger, "Trying {}connection: `{}` and `{}`",
                           if of {""} else {"dis"}, oname, iname);
                    as_inactive!(cli, logger, {
                        if cli.ports(Some(&iname), None, j::port_flags::IS_INPUT).len() == 1 &&
                            cli.ports(Some(&oname), None, j::port_flags::IS_OUTPUT).len() == 1 {
                            cli
                                .connect_ports_by_name_if(&of, &oname, &iname)
                                .log_err(&logger)
                                .map_err(|e| { warn!(logger, "FAILED! Rescheduling!"); e })
                                .retry_if_fail_in(100, JaconSignals::TryConnection(of, oname, iname), &tSig);
                        } else {
                            debug!(logger, "One or both of ports: `{}` and `{}` does not exist!", oname, iname);
                        }

                    });

                }
                JaconSignals::SetConnectionCheck(of) => { disable_check_connections = !of },
                JaconSignals::CheckConection(oname, iname, connected) => {
                    if disable_check_connections {
                        debug!(logger, "Skipping connection checks");
                    } else {
                        let mut is_fine = !connected;
                        for (oo, iis) in &cons {
                            if &oname == oo && iis.contains(&iname) {
                                is_fine = connected;
                                break;
                            }
                        }

                        let stat = if is_fine {"GOOD"} else {"BAD"};
                        let logger = logger.new(o!("stat" => stat));
                        info!(logger, "Ports {}connected: `{}` and `{}`", if connected {""} else {"dis"}, oname, iname);
                        debug!(logger, "{}: ", stat);
                        if !is_fine {
                            let connecting = !connected;
                            debug!(logger, "Scheduling try {}connection: `{}` and `{}`",
                                   if connecting {""} else {"dis"}, oname, iname);
                            as_inactive!(cli, logger, {
                                tSig.send(JaconSignals::TryConnection(connecting, oname, iname));
                            });
                        } else {
                            debug!(logger, "Doing nothing");
                        }
                    }
                }
                JaconSignals::ReconnectGood => {
                    info!(logger, "Reconnecting all good");
                    as_inactive!(cli, logger, {
                        for (oo, iis) in &cons { for ii in iis {
                                cli.connect_ports_by_name(&oo, &ii);
                        } }
                    });
                }
                JaconSignals::DisconnectAll => {
                    info!(logger, "Disconnecting all");
                    tSig.send(JaconSignals::SetConnectionCheck(false));
                    as_inactive!(cli, logger, {
                        for ii in cli.ports(None, None, j::port_flags::IS_INPUT) {
                            cli.port_by_name(&ii).unwrap().disconnect();
                        }
                    });
                    tSig.send(JaconSignals::SetConnectionCheck(true));
                }
            }
        }
    });

    let tSig = _tSig.clone();
    jclient.hook(jam::CB::ports_connected(
        Box::new(move |c, o, i, connected| {
            let oname = c.port_name_by_id(o).unwrap(); // out
            let iname = c.port_name_by_id(i).unwrap(); // in

            tSig.send(JaconSignals::CheckConection(oname, iname, connected));
        })
    ));
    let tSig = _tSig.clone();
    let logger = log.clone();
    jclient.hook(jam::CB::port_registration(
        Box::new(move |c, p, of| {
            let pname = c.port_name_by_id(p).unwrap();
            info!(logger, "Port {}registered: `{}`", if of {""} else {"un"}, pname);
            if of {
                tSig.send(JaconSignals::ReconnectPort(pname));
            }
        })
    ));
    let tSig = _tSig.clone();
    jclient.hook(jam::CB::client_reconnection(
        Box::new(move || {
            tSig.send(JaconSignals::DisconnectAll);
            tSig.send(JaconSignals::ReconnectGood);
        })
    ));
    let tSig = _tSig.clone();
    tSig.send(JaconSignals::DisconnectAll);
    tSig.send(JaconSignals::ReconnectGood);
}
