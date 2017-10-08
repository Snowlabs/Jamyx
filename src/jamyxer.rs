extern crate jam;
extern crate slog;
extern crate jack;

use std;
use std::thread;
use std::sync::mpsc::{channel, Sender, Receiver};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::io::Write;
use std::collections::{HashMap, HashSet};

use jam::JackClientUtils;
use jack::prelude as j;

use config;
use server;

use utils::LogError;

type AM<T> = Arc<Mutex<T>>;
type AMAnyClient = AM<jam::AnyClient>;
type AMConfig = AM<config::Config>;


pub struct Port {
    is_mono: bool,
    ports: HashMap<String, j::Port<Box<j::PortSpec>>>,
}
impl Port {
    pub fn register(name: &str, output: bool, mono: bool, cli: &jam::AnyClient) -> Self {
        let mut ports = HashMap::new();
        // let output = spec.jack_flags() == j::port_flags::IS_OUTPUT;
        let spec = if output { Box::new(j::AudioOutSpec) } else { Box::new(j::AudioInSpec) };

        if mono {
            let pn = format!("{} {}{}", name, "M", if output { " Out" } else {""});
            let port = cli.as_inactive().unwrap().register_port(&pn, spec).unwrap();
            ports.insert("M".to_string(), port);
        } else {
            let pnl = format!("{} {}{}", name, "L", if output { " Out" } else {""});
            let pnr = format!("{} {}{}", name, "R", if output { " Out" } else {""});
            let portl = cli.as_inactive().unwrap().register_port(&pnl, spec).unwrap();
            let portr = cli.as_inactive().unwrap().register_port(&pnr, spec).unwrap();
            ports.insert("L".to_string(), portl);
            ports.insert("R".to_string(), portr);
        }
        Self {
            is_mono: mono,
            ports,
        }
    }
}

pub struct Patchbay {
    log: slog::Logger,
    cli: AMAnyClient,
    cfg: AMConfig,
    cmd_thread: Option<std::thread::JoinHandle<()>>,
    inputs: AM<HashMap<String, j::Port<j::AudioInSpec>>>,
    outputs: AM<HashMap<String, j::Port<j::AudioOutSpec>>>,
}

impl Patchbay {
    pub fn new(log: slog::Logger, cli: AMAnyClient, cfg: AMConfig) -> Self {
        Patchbay {
            log,
            cli,
            cfg,
            cmd_thread: None,
            inputs: Arc::new(Mutex::new(HashMap::new())),
            outputs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn init(&mut self, jclient: &mut jam::Client) {
        for (ref name, ref config) in &self.cfg.lock().unwrap().mixer.outputs {
            // if config.is_mono() {
            //     let chan_m = self.cli.lock().unwrap().as_inactive().unwrap().register_port(&(**name + &" M"), j::AudioOutSpec);
            // } else {
            //     let chan_l = self.cli.lock().unwrap().as_inactive().unwrap().register_port(&(**name + &" R"), j::AudioOutSpec);
            //     let chan_r = self.cli.lock().unwrap().as_inactive().unwrap().register_port(&(**name + &" L"), j::AudioOutSpec);
            // }
            // self.inputs.lock().unwrap().insert(name.clone(), chan);
            // self.outputs.lock().unwrap().insert(name.clone(), chan);
        }

        // Hook process callback
        let log = self.log.clone();
        jclient.hook(jam::CB::process(Box::new(move |cli, scope| {


            return j::JackControl::Continue;
        })));
    }

}
