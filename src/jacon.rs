extern crate jack;
extern crate jam;
extern crate slog;

use std;
use std::net::TcpStream;
use std::sync::mpsc::{
    channel,
    Receiver,
    Sender,
    RecvError,
    SendError,
};
use std::sync::{
    Arc,
    Mutex,
    RwLock,
    PoisonError,
};
use std::thread;

use serde_json;

use jack as j;
use jam::JackClientUtils;

use config;
use server;

use utils::Connections;
use utils::LogError;

#[derive(Debug)]
pub enum Error {
    RecvError(RecvError),
    SendError,
    JackError(j::Error),
    PoisonError,
    PortNotFound,
    JaconUninitialized,
}

impl From<RecvError> for Error {
    fn from(e: RecvError) -> Self { Error::RecvError(e) }
}
impl<T> From<SendError<T>> for Error {
    fn from(_e: SendError<T>) -> Self { Error::SendError }
}
impl From<j::Error> for Error {
    fn from(e: j::Error) -> Self { Error::JackError(e) }
}
impl<T> From<PoisonError<T>> for Error {
    fn from(_e: PoisonError<T>) -> Self { Error::PoisonError }
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
    fn retry_if_fail_in(&self, u64, Signals, &Sender<Signals>)
        -> Result<(), Error>;
}

impl RetryConnection for Result<(), j::Error> {
    fn retry_if_fail_in(&self, delay: u64, sig: Signals, t_sig: &Sender<Signals>) 
        -> Result<(), Error> {

        if let &Err(ref e) = self {
            match e {
                &j::Error::PortConnectionError(_, _) => {
                    t_sig.send(Signals::RetryIn(delay, Box::new(sig)))?;
                }
                &j::Error::PortDisconnectionError => {
                    // t_sig.send(Signals::RetryIn(delay, Box::new(sig)));
                }
                _ => (),
            };
        }
        Ok(())
    }
}

type AM<T> = Arc<Mutex<T>>;
type AMAnyClient = AM<jam::AnyClient>;

pub struct ConnectionKit {
    log: slog::Logger,
    cli: AMAnyClient,
    cfg: Arc<RwLock<config::Config>>,
    t_sig: Option<Sender<Signals>>,
    t_cmd: Option<Sender<(TcpStream, server::Command)>>,
    sig_thread: Option<std::thread::JoinHandle<()>>,
    cmd_thread: Option<std::thread::JoinHandle<()>>,
    // monitors: AM<Vec<(TcpStream, String, String)>>
}

impl ConnectionKit {
    pub fn new(log: slog::Logger, cli: AMAnyClient, cfg: Arc<RwLock<config::Config>>) -> Self {
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

    pub fn init(&mut self, jclient: &mut jam::Client) -> Result<(), Error> {
        let (_t_sig, r_sig) = channel();
        self.t_sig = Some(_t_sig.clone());

        // Prepare vars for move
        let t_sig = _t_sig.clone();
        let log = self.log.clone();
        let cli = self.cli.clone();
        let cfg = self.cfg.clone();
        // Start signal loop
        self.sig_thread = Some(thread::spawn(move || {
            Self::sig_loop((t_sig, r_sig), log, cli, cfg)
                .expect("jacon signal loop failed");
        }));

        // ===== HOOKS =====

        // Hook ports_connected
        let t_sig = _t_sig.clone();
        jclient.hook(jam::CB::ports_connected(Box::new(
            move |c, o, i, connected| {
                let msg = "port not found in callback (this is impossible!)";

                let oname = c.port_name_by_id(o).expect(msg); // out
                let iname = c.port_name_by_id(i).expect(msg); // in

                t_sig.send(Signals::CheckConection(oname, iname, connected))
                    .expect("sending signal to jacon form callback");
            },
        )));

        // Hook port_registration
        let t_sig = _t_sig.clone();
        let log = self.log.clone();
        jclient.hook(jam::CB::port_registration(Box::new(move |c, p, of| {
            let pname = c
                .port_name_by_id(p)
                .expect("port not found in jack callback (this is impossible)");

            info!(
                log,
                "Port {}registered: `{}`",
                if of { "" } else { "un" },
                pname
            );
            if of {
                t_sig
                    .send(Signals::ReconnectPort(pname))
                    .expect("seding reconnection signal in jack callback");
            }
        })));

        // Hook client_reconnection
        let t_sig = _t_sig.clone();
        jclient.hook(jam::CB::client_reconnection(Box::new(move || {
            let msg = "sending signal to jacon from callback (channel closed)";

            t_sig.send(Signals::DisconnectAll).expect(msg);
            t_sig.send(Signals::ReconnectGood).expect(msg);
        })));

        Ok(())
    }

    fn sig_loop(
        sigs: (Sender<Signals>, Receiver<Signals>),
        log: slog::Logger,
        cli: AMAnyClient,
        config: Arc<RwLock<config::Config>>,
    ) -> Result<(), Error> {
        let (t_sig, r_sig) = sigs;
        let mut disable_check_connections = false;
        loop {
            let sig = r_sig.recv()?;
            match sig {
                Signals::ReconnectPort(port_name) => {
                    info!(log, "Reevaluating connections for port: `{}`", port_name);
                    as_inactive!(cli, log, {
                        let port = cli
                            .port_by_name(&port_name)
                            .ok_or(Error::PortNotFound)?;

                        let is_input = port.flags().contains(j::PortFlags::IS_INPUT);
                        cli.disconnect(&port)?;
                        for (oo, iis) in &config.read()?.connections {
                            for ii in iis {

                                if  (is_input  && ii == &port_name) || 
                                    (!is_input && oo == &port_name) {

                                    t_sig
                                        .send(Signals::TryConnection(
                                                true, 
                                                oo.clone(),
                                                ii.clone())
                                            )?;
                                }
                            }
                        }
                    });
                }
                Signals::RetryIn(delay, sig) => {
                    warn!(log, "{:?} failed and got rescheduled", *sig);
                    debug!(log, "scheduling retry in {} milliseconds...", delay);
                    let t_sig = t_sig.clone();
                    thread::spawn(move || {
                        thread::sleep(std::time::Duration::from_millis(delay));
                        t_sig
                            .send(*sig)
                            .expect("sending signal from callback to jacon");
                    });
                }
                Signals::TryConnection(of, oname, iname) => {
                    debug!(
                        log,
                        "Trying {}connection: `{}` and `{}`",
                        if of { "" } else { "dis" },
                        oname,
                        iname
                    );
                    as_inactive!(cli, log, {
                        if cli.ports(Some(&iname), None, j::PortFlags::IS_INPUT).len() == 1
                            && cli
                                .ports(Some(&oname), None, j::PortFlags::IS_OUTPUT)
                                .len()
                                == 1
                        {
                            cli.connect_ports_by_name_if(&of, &oname, &iname)
                                .log_err(&log)
                                .map_err(|e| {
                                    warn!(log, "FAILED! Rescheduling!");
                                    e
                                }).retry_if_fail_in(
                                    100,
                                    Signals::TryConnection(of, oname, iname),
                                    &t_sig,
                                )?;
                        } else {
                            warn!(
                                log,
                                "One or both of ports: `{}` and `{}` does not exist!", oname, iname
                            );
                        }
                    });
                }
                Signals::SetConnectionCheck(of) => {
                    debug!(
                        log,
                        "=> {}abling connection check",
                        if of { "en" } else { "dis" }
                    );
                    disable_check_connections = !of;
                }
                Signals::CheckConection(oname, iname, connected) => {
                    if disable_check_connections {
                        debug!(log, "Skipping connection checks");
                    } else {
                        let mut is_fine = !connected;
                        for (oo, iis) in &config.read()?.connections {
                            if &oname == oo && iis.contains(&iname) {
                                is_fine = connected;
                                break;
                            }
                        }

                        let stat = if is_fine { "GOOD" } else { "BAD" };
                        let log = log.new(o!("stat" => stat));
                        info!(
                            log,
                            "Ports {}connected: `{}` and `{}`",
                            if connected { "" } else { "dis" },
                            oname,
                            iname
                        );
                        debug!(log, "{}: ", stat);
                        if !is_fine {
                            let connecting = !connected;
                            debug!(
                                log,
                                "Scheduling try {}connection: `{}` and `{}`",
                                if connecting { "" } else { "dis" },
                                oname,
                                iname
                            );
                            t_sig
                                .send(Signals::TryConnection(
                                        connecting,
                                        oname,
                                        iname
                                        ))?;
                        } else {
                            debug!(log, "Doing nothing");
                        }
                    }
                }
                Signals::ReconnectGood => {
                    info!(log, "Reconnecting all good");
                    for (oo, iis) in &config.read()?.connections {
                        for ii in iis {
                            t_sig
                                .send(Signals::TryConnection(
                                        true,
                                        oo.clone(),
                                        ii.clone()
                                        ))?;
                        }
                    }
                }
                Signals::DisconnectAll => {
                    info!(log, "Disconnecting all");
                    t_sig.send(Signals::SetConnectionCheck(false))?;
                    as_inactive!(cli, log, {
                        for ii in cli.ports(None, None, j::PortFlags::IS_INPUT) {
                            cli.disconnect(
                                &cli
                                .port_by_name(&ii)
                                .ok_or(Error::PortNotFound)?
                                )?;
                        }
                    });
                    t_sig.send(Signals::SetConnectionCheck(true))?;
                }
            }
        }
    }

    pub fn start(&mut self) -> Result<(), Error> {
        let t_sig = self.t_sig
            .as_ref()
            .ok_or(Error::JaconUninitialized)?
            .clone();

        t_sig.send(Signals::DisconnectAll)?;
        t_sig.send(Signals::ReconnectGood)?;

        let (_t_cmd, r_cmd) = channel();
        self.t_cmd = Some(_t_cmd.clone());
        // let t_cmd = _t_cmd.clone();
        let cfg = self.cfg.clone();
        let log = self.log.clone();
        self.cmd_thread = Some(thread::spawn(move || {
            loop {
                let (mut stream, command): (TcpStream, server::Command) =
                                            r_cmd.recv().unwrap();

                match command.cmd.as_str() {
                    "con" | "dis" | "tog" => {
                        let iname = command.opts[0].clone();
                        let oname = command.opts[1].clone();

                        let connecting = match command.cmd.as_str() {
                            "con" => true,
                            "dis" => false,
                            "tog" | _ => {
                                !cfg.read().unwrap().connections.is_connected(&iname, &oname)
                            }
                        };

                        // let msg = format!("{}connected `{}` and `{}`",
                        //                   if connecting {""} else {"dis"}, iname, oname);

                        // Perform the (dis)connection
                        cfg.write()
                            .unwrap()
                            .connections
                            .connect(connecting, &iname, &oname);
                        t_sig
                            .send(Signals::TryConnection(
                                connecting,
                                iname.clone(),
                                oname.clone(),
                            )).unwrap();

                        server::write_response(
                            &log,
                            &server::Response {
                                ret: 0,
                                msg: &format!("{}connection", if connecting { "" } else { "dis" }),
                                obj: json!({
                                "output_name": &oname,
                                "input_name": &iname,
                            }),
                            },
                            &mut stream,
                        );
                        drop(stream)
                        // let _ = stream.write(msg.as_bytes());
                        // let _ = stream.write(b"\n");
                        // let _ = stream.flush().log_err(&log);
                        // info!(log, "{}", msg);
                    }
                    _ => {
                        server::write_response(
                            &log,
                            &server::Response {
                                ret: 1,
                                msg: "Bad command!",
                                obj: serde_json::Value::Null,
                            },
                            &mut stream,
                        );
                        drop(stream);
                        // let msg = format!("Bad command: `{}`", command.cmd);
                        // let _ = stream.write(msg.as_bytes());
                        // let _ = stream.write(b"\n");
                        // let _ = stream.flush().log_err(&log);
                        // error!(log, "{}", msg);
                    }
                }
            }
        }));

        Ok(())
    }

    pub fn get_cmd_sender(&self) -> Option<&Sender<(TcpStream, server::Command)>> {
        return self.t_cmd.as_ref();
    }
}
