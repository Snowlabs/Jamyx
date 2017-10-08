extern crate jack;
extern crate jam;
extern crate slog;

use std;
use std::thread;
use std::sync::mpsc::{channel, Sender, Receiver};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::io::Write;
use std::collections::{HashMap, HashSet};

use jack::prelude as j;
use jam::JackClientUtils;

use config;
use server;

use utils::LogError;


trait Connections {
    fn connect(&mut self, bool, &str, &str);
    fn is_connected(&self, &str, &str) -> bool;
}

impl Connections for HashMap<String, HashSet<String>> {
    fn connect(&mut self, of: bool, oname: &str, iname: &str) {
        if of {
            self.entry(oname.to_string()).or_insert(HashSet::new()).insert(iname.to_string());
        } else {
            // self.get(iname).as_mut().map()
            let mut erase = false;
            match self.get_mut(oname) {
                Some(os) => {
                    os.remove(iname);
                    erase = os.is_empty();
                },
                None => {},
            }
            if erase {
                self.remove(oname);
            }
        }
    }

    fn is_connected(&self, oname: &str, iname: &str) -> bool {
        self.contains_key(oname) && self[oname].contains(iname)
    }
}


#[derive(Debug)]
pub enum Signals {
    CheckConection(String, String, bool),
    SetConnectionCheck(bool),
    DisconnectAll,
    ReconnectGood,

    RetryIn(u64, Box<Signals>),
    TryConnection(bool, String, String),

    ReconnectPort(String),

    // Connect(String, String, bool),
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

type AM<T> = Arc<Mutex<T>>;
type AMAnyClient = AM<jam::AnyClient>;
type AMConfig = AM<config::Config>;

pub struct ConnectionKit {
    log: slog::Logger,
    cli: AMAnyClient,
    cfg: AMConfig,
    t_sig: Option<Sender<Signals>>,
    pub t_cmd: Option<Sender<(TcpStream, server::Command)>>,
    sig_thread: Option<std::thread::JoinHandle<()>>,
    cmd_thread: Option<std::thread::JoinHandle<()>>,
    // monitors: AM<Vec<(TcpStream, String, String)>>
}

impl ConnectionKit {
    pub fn new(log: slog::Logger, cli: AMAnyClient, cfg: AMConfig) -> Self {
        ConnectionKit {
            log,
            cli,
            cfg,
            t_sig: None,
            t_cmd: None,
            sig_thread: None,
            cmd_thread: None,
            // monitors: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn init(&mut self, jclient: &mut jam::Client) {
        let (_t_sig, r_sig) = channel();
        self.t_sig = Some(_t_sig.clone());

        // Prepare vars for move
        let t_sig = _t_sig.clone();
        let log = self.log.clone();
        let cli = self.cli.clone();
        let cfg = self.cfg.clone();
        // Start signal loop
        self.sig_thread = Some(thread::spawn(move || {
            Self::sig_loop((t_sig, r_sig), log, cli, cfg);
        }));

        // ===== HOOKS =====

        // Hook ports_connected
        let t_sig = _t_sig.clone();
        jclient.hook(jam::CB::ports_connected(
            Box::new(move |c, o, i, connected| {
                let oname = c.port_name_by_id(o).unwrap(); // out
                let iname = c.port_name_by_id(i).unwrap(); // in

                t_sig.send(Signals::CheckConection(oname, iname, connected)).unwrap();
            })
        ));

        // Hook port_registration
        let t_sig = _t_sig.clone();
        let log = self.log.clone();
        jclient.hook(jam::CB::port_registration(
            Box::new(move |c, p, of| {
                let pname = c.port_name_by_id(p).unwrap();
                info!(log, "Port {}registered: `{}`", if of {""} else {"un"}, pname);
                if of {
                    t_sig.send(Signals::ReconnectPort(pname)).unwrap();
                }
            })
        ));

        // Hook client_reconnection
        let t_sig = _t_sig.clone();
        jclient.hook(jam::CB::client_reconnection(
            Box::new(move || {
                t_sig.send(Signals::DisconnectAll).unwrap();
                t_sig.send(Signals::ReconnectGood).unwrap();
            })
        ));

    }

    fn sig_loop(sigs: (Sender<Signals>, Receiver<Signals>), log: slog::Logger, cli: AMAnyClient, config: AMConfig) {
        let (t_sig, r_sig) = sigs;
        let mut disable_check_connections = false;
        loop {
            let sig = r_sig.recv().unwrap();
            match sig {
                Signals::ReconnectPort(port_name) => {
                    info!(log, "Reevaluating connections for port: `{}`",
                          port_name);
                    as_inactive!(cli, log, {
                        let port = cli.port_by_name(&port_name).unwrap();
                        let is_input = port.flags().contains(j::port_flags::IS_INPUT);
                        port.disconnect().unwrap();
                        for (oo, iis) in &config.lock().unwrap().connections { for ii in iis {
                            if (is_input && ii == &port_name) || (!is_input && oo == &port_name) {
                                t_sig.send(Signals::TryConnection(true, oo.clone(), ii.clone())).unwrap();
                            }
                        } }
                    });
                }
                Signals::RetryIn(delay, sig) => {
                    warn!(log, "{:?} failed and got rescheduled", *sig);
                    debug!(log, "scheduling retry in {} milliseconds...", delay);
                    let t_sig = t_sig.clone();
                    thread::spawn(move || {
                        thread::sleep(std::time::Duration::from_millis(delay));
                        t_sig.send(*sig).unwrap();
                    });
                }
                Signals::TryConnection(of, oname, iname) => {
                    debug!(log, "Trying {}connection: `{}` and `{}`",
                           if of {""} else {"dis"}, oname, iname);
                    as_inactive!(cli, log, {
                        if cli.ports(Some(&iname), None, j::port_flags::IS_INPUT).len() == 1 &&
                            cli.ports(Some(&oname), None, j::port_flags::IS_OUTPUT).len() == 1 {
                            cli
                                .connect_ports_by_name_if(&of, &oname, &iname)
                                .log_err(&log)
                                .map_err(|e| { warn!(log, "FAILED! Rescheduling!"); e })
                                .retry_if_fail_in(100, Signals::TryConnection(of, oname, iname), &t_sig);
                        } else {
                            debug!(log, "One or both of ports: `{}` and `{}` does not exist!", oname, iname);
                        }

                    });

                }
                Signals::SetConnectionCheck(of) => {
                    debug!(log, "=> {}abling connection check", if of {"en"} else {"dis"});
                    disable_check_connections = !of;
                },
                Signals::CheckConection(oname, iname, connected) => {
                    if disable_check_connections {
                        debug!(log, "Skipping connection checks");
                    } else {
                        let mut is_fine = !connected;
                        for (oo, iis) in &config.lock().unwrap().connections {
                            if &oname == oo && iis.contains(&iname) {
                                is_fine = connected;
                                break;
                            }
                        }

                        let stat = if is_fine {"GOOD"} else {"BAD"};
                        let log = log.new(o!("stat" => stat));
                        info!(log, "Ports {}connected: `{}` and `{}`", if connected {""} else {"dis"}, oname, iname);
                        debug!(log, "{}: ", stat);
                        if !is_fine {
                            let connecting = !connected;
                            debug!(log, "Scheduling try {}connection: `{}` and `{}`",
                                   if connecting {""} else {"dis"}, oname, iname);
                            t_sig.send(Signals::TryConnection(connecting, oname, iname)).unwrap();
                        } else {
                            debug!(log, "Doing nothing");
                        }
                    }
                }
                Signals::ReconnectGood => {
                    info!(log, "Reconnecting all good");
                    for (oo, iis) in &config.lock().unwrap().connections { for ii in iis {
                            t_sig.send(Signals::TryConnection(true, oo.clone(), ii.clone())).unwrap();
                    } }
                }
                Signals::DisconnectAll => {
                    info!(log, "Disconnecting all");
                    t_sig.send(Signals::SetConnectionCheck(false)).unwrap();
                    as_inactive!(cli, log, {
                        for ii in cli.ports(None, None, j::port_flags::IS_INPUT) {
                            cli.port_by_name(&ii).unwrap().disconnect().unwrap();
                        }
                    });
                    t_sig.send(Signals::SetConnectionCheck(true)).unwrap();
                }
            }
        }
    }

    pub fn start(&mut self) {
        let t_sig = self.t_sig.as_ref().unwrap().clone();

        t_sig.send(Signals::DisconnectAll).unwrap();
        t_sig.send(Signals::ReconnectGood).unwrap();

        let (_t_cmd, r_cmd) = channel();
        self.t_cmd = Some(_t_cmd.clone());
        let t_cmd = _t_cmd.clone();
        let cfg = self.cfg.clone();
        let log = self.log.clone();
        self.cmd_thread = Some(thread::spawn(move || {
            loop {
                let (mut stream, command): (TcpStream, server::Command) = r_cmd.recv().unwrap();
                match command.cmd.as_str() {
                    "con" | "dis" | "tog" => {
                        let iname = command.opts[0].clone();
                        let oname = command.opts[1].clone();

                        let connecting = match command.cmd.as_str() {
                            "con" => true,
                            "dis" => false,
                            "tog" | _ => !cfg.lock().unwrap().connections.is_connected(&iname, &oname),
                        };

                        let msg = format!("{}connected `{}` and `{}`",
                                          if connecting {""} else {"dis"}, iname, oname);

                        // Perform the (dis)connection
                        cfg.lock().unwrap().connections.connect(connecting, &iname, &oname);
                        t_sig.send(Signals::TryConnection(connecting, iname, oname));

                        let _ = stream.write(msg.as_bytes());
                        let _ = stream.write(b"\n");
                        let _ = stream.flush().log_err(&log);
                        info!(log, "{}", msg);
                    }
                    _ => {
                        let msg = format!("Bad command: `{}`", command.cmd);
                        let _ = stream.write(msg.as_bytes());
                        let _ = stream.write(b"\n");
                        let _ = stream.flush().log_err(&log);
                        error!(log, "{}", msg);
                    }
                }
            }
        }));
    }
}

