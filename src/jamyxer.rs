extern crate jam;
extern crate slog;
extern crate jack;

use std;
use std::thread;
use std::sync::mpsc::{channel, Sender, Receiver};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, RwLock};
use std::io::Write;
use std::collections::{HashMap, HashSet};

use jam::JackClientUtils;
use jack::prelude as j;

use config;
use server;

use utils::LogError;
use utils::Connections;

type AM<T> = Arc<Mutex<T>>;
type AMAnyClient = AM<jam::AnyClient>;


pub struct Port {
    is_mono: bool,
    is_output: bool,
    ports: HashMap<String, j::Port<jam::AnySpec>>,
}

impl Port {
    pub fn register(name: &str, output: bool, mono: bool, cli: &jam::AnyClient) -> Self {
        let mut ports = HashMap::new();
        let spec = if output { jam::AnySpec::AudioOut } else { jam::AnySpec::AudioIn };

        if mono {
            let pn = format!("{} M", name);
            let port = cli.as_inactive().unwrap().register_port(&pn, spec).unwrap();
            ports.insert("M".to_string(), port);
        } else {
            let pnl = format!("{} L", name);
            let pnr = format!("{} R", name);
            let portl = cli.as_inactive().unwrap().register_port(&pnl, spec).unwrap();
            let portr = cli.as_inactive().unwrap().register_port(&pnr, spec).unwrap();
            ports.insert("L".to_string(), portl);
            ports.insert("R".to_string(), portr);
        }
        Self {
            is_mono: mono,
            is_output: output,
            ports,
        }
    }

    pub fn register_output(name: &str, mono: bool, cli: &jam::AnyClient) -> Self {
        Self::register(name, true, mono, cli)
    }

    pub fn register_input(name: &str, mono: bool, cli: &jam::AnyClient) -> Self {
        Self::register(name, false, mono, cli)
    }

    pub fn copy_from(&mut self, other: &Self, vol: f32, balance: (f32, f32), ps: &j::ProcessScope, log: &slog::Logger) {
        if !self.is_output { return; /* TODO: Panic here or something */ }

        if self.is_mono {
                let ref mut port = self.ports.get_mut("M").unwrap();
                let mut oport = jam::AnyAudioOutPort::new(port, ps);
                if other.is_mono {
                    // === MONO TO MONO ===
                    let other_p = jam::AnyAudioInPort::new(&other.ports["M"], ps);
                    // oport.clone_from_slice(&other_p);
                    for (i, e) in oport.iter_mut().enumerate() {
                        *e = other_p[i] * vol;
                    }
                } else {
                    // === STEREO TO MONO ===
                    let other_p_l = jam::AnyAudioInPort::new(&other.ports["L"], ps);
                    let other_p_r = jam::AnyAudioInPort::new(&other.ports["R"], ps);
                    // oport.clone_from_slice(&other_p_l);
                    for (i, e) in oport.iter_mut().enumerate() {
                        *e = other_p_l[i] * vol;
                    }
                    for (i, e) in oport.iter_mut().enumerate() {
                        *e *= other_p_r[i] * vol; // Multiply???
                    }
            }
        } else {
            // to left channel
            {
                let ref mut port_l = self.ports.get_mut("L").unwrap();
                let mut oport_l = jam::AnyAudioOutPort::new(port_l, ps);
                if other.is_mono {
                    // === MONO TO STEREO ===
                    let other_p = jam::AnyAudioInPort::new(&other.ports["M"], ps);
                    // oport_l.copy_from_slice(&other_p);
                    for (i, e) in oport_l.iter_mut().enumerate() {
                        *e = other_p[i] * vol * balance.0;
                    }
                } else {
                    // === STEREO TO STEREO ===
                    let other_p_l = jam::AnyAudioInPort::new(&other.ports["L"], ps);
                    // oport_l.copy_from_slice(&other_p_l);
                    for (i, e) in oport_l.iter_mut().enumerate() {
                        *e = other_p_l[i] * vol * balance.0;
                    }
                }
            }
            // to right channel
            {
                let ref mut port_r = self.ports.get_mut("R").unwrap();
                let mut oport_r = jam::AnyAudioOutPort::new(port_r, ps);
                if other.is_mono {
                    // === MONO TO STEREO ===
                    let other_p = jam::AnyAudioInPort::new(&other.ports["M"], ps);
                    // oport_r.copy_from_slice(&other_p);
                    for (i, e) in oport_r.iter_mut().enumerate() {
                        *e = other_p[i] * vol * balance.1;
                    }
                } else {
                    // === STEREO TO STEREO ===
                    let other_p_r = jam::AnyAudioInPort::new(&other.ports["R"], ps);
                    // oport_r.copy_from_slice(&other_p_r);
                    for (i, e) in oport_r.iter_mut().enumerate() {
                        *e = other_p_r[i] * vol * balance.1;
                    }
                }
            }
        }
    }
}


pub struct Patchbay {
    log: slog::Logger,
    cli: AMAnyClient,
    cfg: Arc<RwLock<config::Config>>,
    inputs: AM<HashMap<String, Port>>,
    input_outs: AM<HashMap<String, Port>>,
    outputs: AM<HashMap<String, Port>>,

    pub t_cmd: Option<Sender<(TcpStream, server::Command)>>,
    cmd_thread: Option<std::thread::JoinHandle<()>>,
}

impl Patchbay {
    pub fn new(log: slog::Logger, cli: AMAnyClient, cfg: Arc<RwLock<config::Config>>) -> Self {
        Patchbay {
            log,
            cli,
            cfg,
            inputs: Arc::new(Mutex::new(HashMap::new())),
            input_outs: Arc::new(Mutex::new(HashMap::new())),
            outputs: Arc::new(Mutex::new(HashMap::new())),

            t_cmd: None,
            cmd_thread: None,
        }
    }

    pub fn init(&mut self, jclient: &mut jam::Client) {
        for (ref name, ref config) in &self.cfg.read().unwrap().mixer.inputs {
            self.inputs.lock().unwrap().insert(
                name.clone().to_owned(),
                Port::register_input(&name, config.is_mono(), &self.cli.lock().unwrap())
                );
            self.input_outs.lock().unwrap().insert(
                name.clone().to_owned(),
                Port::register_output(&format!("{} Out", name), config.is_mono(), &self.cli.lock().unwrap())
                );
        }

        for (ref name, ref config) in &self.cfg.read().unwrap().mixer.outputs {
            self.outputs.lock().unwrap().insert(
                name.clone().to_owned(),
                Port::register_output(&name, config.is_mono(), &self.cli.lock().unwrap())
                );
        }

        // Hook process callback
        let log = self.log.clone();
        let ins = self.inputs.clone();
        let ios = self.input_outs.clone();
        let outs = self.outputs.clone();
        let cfg = self.cfg.clone();
        jclient.hook(jam::CB::process(Box::new(move |cli, scope| {
            // debug!(log, "test: {:?}", cfg.lock().unwrap().mixer.outputs);
            let combine_balance = |a: (f32, f32), b: (f32, f32)| (a.0 * b.0, a.1 * b.1);

            // let cfg = cfg.lock().unwrap().clone();
            for (ref name, ref config) in &cfg.read().unwrap().mixer.inputs {
                ios.lock().unwrap().get_mut(*name).unwrap().copy_from(
                    &ins.lock().unwrap()[*name],
                    config.get_vol() as f32,
                    config.get_balance_pair(),
                    &scope,
                    &log);
            }
            for (ref o, ref is) in &cfg.read().unwrap().mixer.connections {
                for ref i in is.iter() {
                    let mut outs = outs.lock().unwrap();
                    let ins = ins.lock().unwrap();
                    let cfg = cfg.read().unwrap();

                    outs.get_mut(*o).unwrap().copy_from(
                        &ins[*i],
                        cfg.mixer.inputs[*i].get_vol() as f32 *
                        cfg.mixer.outputs[*o].get_vol() as f32,

                        combine_balance(
                            cfg.mixer.inputs[*i].get_balance_pair(),
                            cfg.mixer.outputs[*o].get_balance_pair()
                        ),
                        &scope,
                        &log);
                }
            }

            return j::JackControl::Continue;
        })));
    }

    pub fn start(&mut self) {
        let (_t_cmd, r_cmd) = channel();
        let cfg = self.cfg.clone();
        let log = self.log.clone();

        self.t_cmd = Some(_t_cmd.clone());
        self.cmd_thread = Some(thread::spawn(move || {
            loop {
                let (mut stream, command): (TcpStream, server::Command) = r_cmd.recv().unwrap();
                let get_ptype = |pt: &String| match &**pt {
                                    "input"|"in"|"i" => false,
                                    "output"|"out"|"o"|_ => true, // TODO: Handle bad args to server commands
                                };

                match command.cmd.as_str() {
                    "con" | "dis" | "tog" => {
                        let iname = command.opts[0].clone();
                        let oname = command.opts[1].clone();

                        let connecting = match command.cmd.as_str() {
                            "con" => true,
                            "dis" => false,
                            "tog" | _ => !cfg.read().unwrap().mixer.connections.is_connected(&oname, &iname),
                        };



                        // Perform the (dis)connection
                        cfg.write().unwrap().mixer.connections.connect(connecting, &oname, &iname);

                        let mut msg = format!("{0}connected `{1}` and `{2}`\nCurrently connected ports for output: `{2}`:",
                                          if connecting {""} else {"dis"}, iname, oname);
                        for i in &cfg.read().unwrap().mixer.connections[&oname] {
                            msg = format!("{}\n- {}", msg, i);
                        }


                        let _ = stream.write(msg.as_bytes());
                        let _ = stream.write(b"\n");
                        let _ = stream.flush().log_err(&log);
                        info!(log, "{}", msg);

                    }
                    "get" => {
                        let what = command.opts[0].clone();
                        match &*what {
                            "volule"|"vol"|"v" => {
                                let ptype = command.opts[1].clone();
                                let is_output = get_ptype(&ptype);

                                let p_name = command.opts[2].clone();
                                let vol = cfg.read().unwrap().mixer.get_vol(is_output, &p_name);

                                let msg;
                                match vol {
                                    Ok(v) => { msg = format!("{}", v); },
                                    Err(_) => { msg = "Error: port not found!".to_string() },
                                }
                                // let msg = format!("{:?}", vol);

                                let _ = stream.write(msg.as_bytes());
                                let _ = stream.write(b"\n");
                                let _ = stream.flush().log_err(&log);
                                info!(log, "Vol of {}: `{}`: {}", ptype, p_name, msg);
                            }
                            "connections"|"cons"|"con"|"c" => {
                                let ptype = command.opts[1].clone();
                                let is_output = get_ptype(&ptype);
                                let p_name = command.opts[2].clone();

                                let cons = cfg.read().unwrap().mixer.get_connected(is_output, &p_name);

                                let mut msg;
                                match cons {
                                    Ok(c) => {
                                        msg = format!("Currently connected ports for {}: `{}`:",
                                                      if is_output {"output"} else {"input"}, p_name);
                                        for p in &c {
                                            msg = format!("{}\n- {}", msg, p);
                                        }
                                    }
                                    Err(_) => { msg = "Error: port not found!".to_string() },
                                }


                                let _ = stream.write(msg.as_bytes());
                                let _ = stream.write(b"\n");
                                let _ = stream.flush().log_err(&log);
                                info!(log, "{}", msg);
                            }
                            _ => {}
                        }

                    }
                    /*
                    "mon" => {
                    }
                    "mkp" => {
                    }
                    "rmp" => {
                    }

                     */
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
