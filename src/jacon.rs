extern crate jack;
extern crate jam;
extern crate slog;

use std;
use std::thread;
use std::sync::mpsc::{channel, Sender};

use jack::prelude as j;
use jam::JackClientUtils;

use config;
// use utils;

use utils::LogError;

#[derive(Debug)]
pub enum Signals {
    CheckConection(String, String, bool),
    SetConnectionCheck(bool),
    DisconnectAll,
    ReconnectGood,

    RetryIn(u64, Box<Signals>),
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
    fn retry_if_fail_in(&self, u64, Signals, &Sender<Signals>);
}

impl RetryConnection for Result<(), j::JackErr> {
    fn retry_if_fail_in(&self, delay: u64, sig: Signals, t_sig: &Sender<Signals>) {
        if let &Err(ref e) = self { match e {
                &j::JackErr::PortConnectionError(_,_) => {
                    t_sig.send(Signals::RetryIn(delay, Box::new(sig))).unwrap();
                }
                &j::JackErr::PortDisconnectionError => {
                    // t_sig.send(Signals::RetryIn(delay, Box::new(sig)));
                }, _ => ()
        } }
    }
}


pub fn setup(log: slog::Logger, jclient: &mut jam::Client, config: &config::Config) -> Sender<Signals> {
    let cons = config.connections.clone();
    let cli = jclient.jclient.clone(); // clones the Arc
    let logger = log.new(o!());
    let (_t_sig, r_sig) = channel();

    let t_sig = _t_sig.clone();
    thread::spawn(move || {
        let mut disable_check_connections = false;
        loop {
            let sig = r_sig.recv().unwrap();
            match sig {
                Signals::ReconnectPort(port_name) => {
                    info!(logger, "Reevaluating connections for port: `{}`",
                          port_name);
                    as_inactive!(cli, logger, {
                        let port = cli.port_by_name(&port_name).unwrap();
                        let is_input = port.flags().contains(j::port_flags::IS_INPUT);
                        port.disconnect().unwrap();
                        for (oo, iis) in &cons { for ii in iis {
                            if (is_input && ii == &port_name) || (!is_input && oo == &port_name) {
                                t_sig.send(Signals::TryConnection(true, oo.clone(), ii.clone())).unwrap();
                            }
                        } }
                    });
                }
                Signals::RetryIn(delay, sig) => {
                    warn!(logger, "{:?} failed and got rescheduled", *sig);
                    debug!(logger, "scheduling retry in {} milliseconds...", delay);
                    let t_sig = t_sig.clone();
                    thread::spawn(move || {
                        thread::sleep(std::time::Duration::from_millis(delay));
                        t_sig.send(*sig).unwrap();
                    });
                }
                Signals::TryConnection(of, oname, iname) => {
                    debug!(logger, "Trying {}connection: `{}` and `{}`",
                           if of {""} else {"dis"}, oname, iname);
                    as_inactive!(cli, logger, {
                        if cli.ports(Some(&iname), None, j::port_flags::IS_INPUT).len() == 1 &&
                            cli.ports(Some(&oname), None, j::port_flags::IS_OUTPUT).len() == 1 {
                            cli
                                .connect_ports_by_name_if(&of, &oname, &iname)
                                .log_err(&logger)
                                .map_err(|e| { warn!(logger, "FAILED! Rescheduling!"); e })
                                .retry_if_fail_in(100, Signals::TryConnection(of, oname, iname), &t_sig);
                        } else {
                            debug!(logger, "One or both of ports: `{}` and `{}` does not exist!", oname, iname);
                        }

                    });

                }
                Signals::SetConnectionCheck(of) => {
                    debug!(logger, "=> {}abling connection check", if of {"en"} else {"dis"});
                    disable_check_connections = !of;
                },
                Signals::CheckConection(oname, iname, connected) => {
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
                            t_sig.send(Signals::TryConnection(connecting, oname, iname)).unwrap();
                        } else {
                            debug!(logger, "Doing nothing");
                        }
                    }
                }
                Signals::ReconnectGood => {
                    info!(logger, "Reconnecting all good");
                    for (oo, iis) in &cons { for ii in iis {
                            t_sig.send(Signals::TryConnection(true, oo.clone(), ii.clone())).unwrap();
                    } }
                }
                Signals::DisconnectAll => {
                    info!(logger, "Disconnecting all");
                    t_sig.send(Signals::SetConnectionCheck(false)).unwrap();
                    as_inactive!(cli, logger, {
                        for ii in cli.ports(None, None, j::port_flags::IS_INPUT) {
                            cli.port_by_name(&ii).unwrap().disconnect().unwrap();
                        }
                    });
                    t_sig.send(Signals::SetConnectionCheck(true)).unwrap();
                }
            }
        }
    });

    let t_sig = _t_sig.clone();
    jclient.hook(jam::CB::ports_connected(
        Box::new(move |c, o, i, connected| {
            let oname = c.port_name_by_id(o).unwrap(); // out
            let iname = c.port_name_by_id(i).unwrap(); // in

            t_sig.send(Signals::CheckConection(oname, iname, connected)).unwrap();
        })
    ));
    let t_sig = _t_sig.clone();
    let logger = log.clone();
    jclient.hook(jam::CB::port_registration(
        Box::new(move |c, p, of| {
            let pname = c.port_name_by_id(p).unwrap();
            info!(logger, "Port {}registered: `{}`", if of {""} else {"un"}, pname);
            if of {
                t_sig.send(Signals::ReconnectPort(pname)).unwrap();
            }
        })
    ));
    let t_sig = _t_sig.clone();
    jclient.hook(jam::CB::client_reconnection(
        Box::new(move || {
            t_sig.send(Signals::DisconnectAll).unwrap();
            t_sig.send(Signals::ReconnectGood).unwrap();
        })
    ));
    let t_sig = _t_sig.clone();
    t_sig
}

pub fn start(t_sig: Sender<Signals>) {
    t_sig.send(Signals::DisconnectAll).unwrap();
    t_sig.send(Signals::ReconnectGood).unwrap();
}
