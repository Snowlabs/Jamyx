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

use serde_json::Value;

use config;
use server;

use utils::LogError;

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

    pub fn zero(&mut self, ps: &j::ProcessScope) {
        if !self.is_output { return; /* TODO: Panic here or something */ }
        match self.is_mono {
            true => {
                let ref mut port = self.ports.get_mut("M").unwrap();
                let mut oport = jam::AnyAudioOutPort::new(port, ps);
                for e in oport.iter_mut() { *e = 0.0 as f32; }
            }
            false => {
                {
                    let ref mut port_l = self.ports.get_mut("L").unwrap();
                    let mut oport_l = jam::AnyAudioOutPort::new(port_l, ps);
                    for e in oport_l.iter_mut() { *e = 0.0 as f32; }
                }
                {
                    let ref mut port_r = self.ports.get_mut("R").unwrap();
                    let mut oport_r = jam::AnyAudioOutPort::new(port_r, ps);
                    for e in oport_r.iter_mut() { *e = 0.0 as f32; }
                }
            }
        }
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
                        *e += other_p[i] * vol;
                    }
                } else {
                    // === STEREO TO MONO ===
                    let other_p_l = jam::AnyAudioInPort::new(&other.ports["L"], ps);
                    let other_p_r = jam::AnyAudioInPort::new(&other.ports["R"], ps);
                    // oport.clone_from_slice(&other_p_l);
                    for (i, e) in oport.iter_mut().enumerate() {
                        *e += other_p_l[i] * vol;
                    }
                    for (i, e) in oport.iter_mut().enumerate() {
                        *e += other_p_r[i] * vol; // Multiply???
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
                        *e += other_p[i] * vol * balance.0;
                    }
                } else {
                    // === STEREO TO STEREO ===
                    let other_p_l = jam::AnyAudioInPort::new(&other.ports["L"], ps);
                    // oport_l.copy_from_slice(&other_p_l);
                    for (i, e) in oport_l.iter_mut().enumerate() {
                        *e += other_p_l[i] * vol * balance.0;
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
                        *e += other_p[i] * vol * balance.1;
                    }
                } else {
                    // === STEREO TO STEREO ===
                    let other_p_r = jam::AnyAudioInPort::new(&other.ports["R"], ps);
                    // oport_r.copy_from_slice(&other_p_r);
                    for (i, e) in oport_r.iter_mut().enumerate() {
                        *e += other_p_r[i] * vol * balance.1;
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

    monitor_port: Option<AM<Port>>,

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

            monitor_port: None,

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

        let monitor_port = Arc::new(Mutex::new(Port::register_output("MONITOR", false, &self.cli.lock().unwrap())));
        self.monitor_port = Some(monitor_port.clone());

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
            for (ref i, ref config) in &cfg.read().unwrap().mixer.inputs {
                ios.lock().unwrap().get_mut(*i).unwrap().zero(&scope);
                ios.lock().unwrap().get_mut(*i).unwrap().copy_from(
                    &ins.lock().unwrap()[*i],
                    config.get_vol() as f32,
                    config.get_balance_pair(),
                    &scope,
                    &log);
            }

            for (ref o, ref is) in &cfg.read().unwrap().mixer.connections {
                outs.lock().unwrap().get_mut(*o).unwrap().zero(&scope);
                for ref i in is.iter() {
                    let cfg = cfg.read().unwrap();

                    outs.lock().unwrap().get_mut(*o).unwrap().copy_from(
                        &ins.lock().unwrap()[*i],
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

            {
                let is_output = !cfg.read().unwrap().mixer.monitor.is_input;
                // let config = cfg.read().unwrap().mixer.get_vol
                let ref moned_port_name = cfg.read().unwrap().mixer.monitor.channel;
                let ref moned_port = match cfg.read().unwrap().mixer.monitor.is_input {
                    true => ins.lock().unwrap(),
                    false => outs.lock().unwrap(),
                }[moned_port_name];
                // debug!(log, "copying mon... {} to monitor port", moned_port_name);
                monitor_port.lock().unwrap().zero(&scope);
                monitor_port.lock().unwrap().copy_from(
                        moned_port,
                        cfg.read().unwrap().mixer.get_vol(is_output, moned_port_name).unwrap(),
                        cfg.read().unwrap().mixer.get_bal_pair(is_output, moned_port_name).unwrap(),
                        &scope,
                        &log);
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
                            "tog" | _ => !cfg.read().unwrap().mixer.is_connected(&oname, &iname),
                        };



                        // Perform the (dis)connection
                        cfg.write().unwrap().mixer.connect(connecting, &oname, &iname);

                        if cfg.read().unwrap().mixer.port_exists(true, &oname.to_string()) {
                            cfg.read().unwrap().mixer.write_info_response(
                                true, &oname.to_string(), &mut stream, &log
                                );
                        } else {
                            server::write_response(&log, &server::Response {
                                ret: 2, msg: "Port not found!", obj: Value::Null
                            }, &mut stream);
                        }
                        drop(stream);

                        // let mut msg = format!("{0}connected `{1}` and `{2}`\nCurrently connected ports for output: `{2}`:",
                        //                   if connecting {""} else {"dis"}, iname, oname);
                        // for i in &cfg.read().unwrap().mixer.connections[&oname] {
                        //     msg = format!("{}\n- {}", msg, i);
                        // }


                        // let _ = stream.write(msg.as_bytes());
                        // let _ = stream.write(b"\n");
                        // let _ = stream.flush().log_err(&log);
                        // info!(log, "{}", msg);

                    }
                    "get" => {
                        // get all i/os
                        match command.opts[0].as_str() {
                            "ports"|"channels" => {
                                let cfg = cfg.read().unwrap();
                                // let ref names: Vec<&String> = match is_output {
                                //     true => &cfg.mixer.outputs,
                                //     false => &cfg.mixer.inputs,
                                // }.keys().collect();
                                let inames: Vec<&String> = cfg.mixer.inputs.keys().collect();
                                let mut inputs: Vec<Value> = Vec::new();
                                for iname in inames {
                                    inputs.push(cfg.mixer.get_port_info(false, iname).unwrap());
                                }

                                let onames: Vec<&String> = cfg.mixer.outputs.keys().collect();
                                let mut outputs: Vec<Value> = Vec::new();
                                for oname in onames {
                                    outputs.push(cfg.mixer.get_port_info(true, oname).unwrap());
                                }

                                server::write_response(&log, &server::Response {
                                    ret: 0, msg: "channels", obj: json!({ "inputs": inputs, "outputs": outputs, })
                                }, &mut stream);
                                drop(stream);
                            }
                            "monitor"|"mon" => {
                                let cfg = cfg.read().unwrap();
                                cfg.mixer.write_info_response(!cfg.mixer.monitor.is_input, &cfg.mixer.monitor.channel, &mut stream, &log);
                                drop(stream);
                            }
                            _ => {
                                let ptype = command.opts[0].clone();
                                let is_output = get_ptype(&ptype);
                                let p_name = command.opts[1].clone();

                                if cfg.read().unwrap().mixer.port_exists(is_output, &p_name.to_string()) {
                                    cfg.read().unwrap().mixer.write_info_response(
                                        is_output, &p_name.to_string(), &mut stream, &log
                                        );
                                } else {
                                    server::write_response(&log, &server::Response {
                                        ret: 2, msg: "Port not found!", obj: Value::Null
                                    }, &mut stream);
                                }
                                drop(stream);
                            }
                        }

                        // let what = command.opts[0].clone();
                        // match &*what {
                        //     "volule"|"vol"|"v"
                        //     |"balance"|"bal"|"b" => {
                        //         let ptype = command.opts[1].clone();
                        //         let is_output = get_ptype(&ptype);

                        //         let p_name = command.opts[2].clone();
                        //         let val = match &*what {
                        //             "volule"|"vol"|"v" => cfg.read().unwrap().mixer.get_vol(is_output, &p_name),
                        //             "balance"|"bal"|"b"|_ => cfg.read().unwrap().mixer.get_bal(is_output, &p_name),
                        //         };
                        //         // let vol = cfg.read().unwrap().mixer.get_vol(is_output, &p_name);

                        //         let msg;
                        //         match val {
                        //             Ok(v) => { msg = format!("{}", v); },
                        //             Err(_) => { msg = "Error: port not found!".to_string() },
                        //         }
                        //         // let msg = format!("{:?}", vol);

                        //         let _ = stream.write(msg.as_bytes());
                        //         let _ = stream.write(b"\n");
                        //         let _ = stream.flush().log_err(&log);
                        //         info!(log, "{} of {}: `{}`: {}", what, ptype, p_name, msg);
                        //     }
                        //     "connections"|"cons"|"con"|"c" => {
                        //         let ptype = command.opts[1].clone();
                        //         let is_output = get_ptype(&ptype);
                        //         let p_name = command.opts[2].clone();

                        //         let cons = cfg.read().unwrap().mixer.get_connected(is_output, &p_name);

                        //         let mut msg;
                        //         match cons {
                        //             Ok(c) => {
                        //                 msg = format!("Currently connected ports for {}: `{}`:",
                        //                               if is_output {"output"} else {"input"}, p_name);
                        //                 for p in &c {
                        //                     msg = format!("{}\n- {}", msg, p);
                        //                 }
                        //             }
                        //             Err(_) => { msg = "Error: port not found!".to_string() },
                        //         }


                        //         let _ = stream.write(msg.as_bytes());
                        //         let _ = stream.write(b"\n");
                        //         let _ = stream.flush().log_err(&log);
                        //         info!(log, "{}", msg);
                        //     }
                        //     _ => {}
                        // }

                    }
                    "mon" => {
                        let what = command.opts[0].clone();
                        match &*what {
                            "volume"|"vol"|"v"
                            |"connections"|"cons"|"con"|"c"
                            |"balance"|"bal"|"b"=> {
                                let ptype = command.opts[1].clone();
                                let is_output = get_ptype(&ptype);
                                let p_name = command.opts[2].clone();
                                let h_name = match &*what {
                                    "volume"|"vol"|"v" => {
                                        if is_output { "output_vol" } else { "input_vol" }
                                    },
                                    "connections"|"cons"|"con"|"c" => {
                                        if is_output { "output_con" } else { "input_con" }
                                    },
                                    "balance"|"bal"|"b"|_ => {
                                        if is_output { "output_bal" } else { "input_bal" }
                                    },
                                }.to_owned();

                                info!(log, "Hooking {} monitor", h_name);
                                cfg.write().unwrap().mixer.hook(h_name, p_name, stream, log.clone());
                            }
                            _ => {}
                        }
                    }
                    "set" => {
                        let what = command.opts[0].clone();
                        match &*what {
                            "volule"|"vol"|"v"
                            |"balance"|"bal"|"b" => {
                                let ptype = command.opts[1].clone();
                                let is_output = get_ptype(&ptype);

                                let p_name = command.opts[2].clone();
                                let val = command.opts[3].clone().parse().unwrap();
                                let ret = match &*what {
                                    "volule"|"vol"|"v" => cfg.write().unwrap().mixer.set_vol(is_output, &p_name.clone(), val),
                                    "balance"|"bal"|"b"|_ => cfg.write().unwrap().mixer.set_bal(is_output, &p_name.clone(), val),
                                };
                                // let vol = cfg.read().unwrap().mixer.get_vol(is_output, &p_name);

                                // let msg;
                                match ret {
                                    Ok(_) => {
                                        cfg.read().unwrap().mixer.write_info_response(
                                            is_output, &p_name.to_string(), &mut stream, &log
                                            );
                                    },
                                    Err(_) => {
                                        server::write_response(&log, &server::Response {
                                            ret: 2, msg: "Port not found!", obj: Value::Null
                                        }, &mut stream);
                                        // msg = "Error: port not found!".to_string()
                                    },
                                }
                                drop(stream);
                                // let msg = format!("{:?}", vol);

                                // let _ = stream.write(msg.as_bytes());
                                // let _ = stream.write(b"\n");
                                // let _ = stream.flush().log_err(&log);
                                // info!(log, "{}", msg);
                            }
                            "monitor"|"mon"|"m" => {
                                let ptype = command.opts[1].clone();
                                let is_output = get_ptype(&ptype);

                                let p_name = command.opts[2].clone();
                                let res = cfg.write().unwrap().mixer.set_mon(is_output, &p_name);
                                match res {
                                    Ok(_) => {
                                        cfg.read().unwrap().mixer.write_info_response(
                                            is_output, &p_name.to_string(), &mut stream, &log
                                            );
                                    },
                                    Err(_) => {
                                        server::write_response(&log, &server::Response {
                                            ret: 2, msg: "Port not found!", obj: Value::Null
                                        }, &mut stream);
                                    },
                                }
                                drop(stream);
                            }
                            _ => {}
                        }

                    }
                    /*
                    "mkp" => {
                    }
                    "rmp" => {
                    }

                     */
                    _ => {
                        server::write_response(
                            &log,
                            &server::Response { ret: 1, msg: "Bad command!", obj: Value::Null, },
                            &mut stream);
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
    }

}
