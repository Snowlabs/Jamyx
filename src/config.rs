extern crate serde;
extern crate serde_json;
extern crate slog;

use std;
use std::error::Error;
use std::io::Read;
use std::collections::{HashMap, HashSet};
use std::net::TcpStream;
use std::io::Write;

use utils::LogError;
use utils::Connections;

use server;

#[derive(Serialize, Deserialize, Debug)]
pub struct PortConfig {
    #[serde(default = "PortConfig::default_vol")]
    pub vol: f32,
    #[serde(default = "PortConfig::default_bal")]
    pub balance: f32,
    #[serde(default = "PortConfig::default_mono")]
    pub mono: bool,
}

impl PortConfig {
    fn default_vol() -> f32 { 100.0 as f32 }
    fn default_bal() -> f32 { 0.0 as f32 }
    fn default_mono() -> bool { false }

    pub fn get_vol(&self) -> f32 { self.vol / 100.0 }

    pub fn get_balance(&self) -> f32 { self.balance }

    pub fn get_balance_pair(&self) -> (f32, f32) {
        let b = self.get_balance();
        (b+1.0, -b+1.0)
    }

    pub fn is_mono(&self) -> bool { self.mono }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MonitorConfig {
    pub channel: String,
    pub is_input: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MixerConfig {
    pub connections: HashMap<String, HashSet<String>>,
    pub outputs: HashMap<String, PortConfig>,
    pub inputs: HashMap<String, PortConfig>,
    pub monitor: MonitorConfig,

    // #[serde(skip)]
    // #[serde(default = "MixerConfig::get_default_hooks")]
    // o_vol_hooks: HashMap<String, Vec<TcpStream>>,
    #[serde(skip)]
    #[serde(default = "MixerConfig::get_default_hooks")]
    mon_hooks: HashMap<String, HashMap<String, Vec<(TcpStream, slog::Logger)>>>,
}

impl MixerConfig {
    pub fn get_default_hooks() -> HashMap<String, HashMap<String, Vec<(TcpStream, slog::Logger)>>>{ HashMap::new() }

    pub fn get_port_info(&self, is_output: bool, name: &String) -> Result<serde_json::Value, ()> {
        if !self.port_exists(is_output, name) { return Err(()); }

        Ok(json!({
            "port":   &*name,
            "ptype":  if is_output {"out"} else {"in"},
            "vol":    self.get_vol(is_output, name)?*100.0,
            "bal":    self.get_bal(is_output, name)?,
            "ismono": self.get_mono(is_output, name)?,
            "cons":   self.get_connected(is_output, name)?,
        }))
    }

    pub fn write_info_response(&self, is_output: bool, name: &String, stream: &mut TcpStream, log: &slog::Logger,) {
        server::write_response(&log, &server::Response {
            ret: 0, msg: &format!("{} info", name), obj: self.get_port_info(is_output, name).unwrap()
        }, stream);
    }

    pub fn get_mono(&self, is_output: bool, name: &String) -> Result<bool, ()> {
        if !self.port_exists(is_output, name) { return Err(()); }

        Ok(if is_output {
            &self.outputs
        } else {
            &self.inputs
        }.get(name).unwrap().is_mono())
    }

    pub fn get_vol(&self, is_output: bool, name: &String) -> Result<f32, ()> {
        if is_output {
            match self.outputs.get(name) {
                Some(o) => Ok(o.get_vol()),
                None => Err(()),
            }
        } else {
            match self.inputs.get(name) {
                Some(i) => Ok(i.get_vol()),
                None => Err(()),
            }
        }
    }

    pub fn set_vol(&mut self, is_output: bool, name: &String, vol: f32) -> Result<(), ()> {
        if !self.port_exists(is_output, name) { return Err(()); }
        // let _vol = serde_json::Number::from_f64(vol as f64);
        // match _vol {
            // Some(_vol) => {
                match is_output {
                    true => &mut self.outputs,
                    false => &mut self.inputs,
                }.get_mut(name).unwrap().vol = vol;
                if let Some(mut hs) = self.mon_hooks
                        .entry(if is_output {"output_vol".to_owned()} else {"input_vol".to_owned()})
                        .or_insert(HashMap::new())
                        .remove(name) {


                    // let msg = format!("{}", vol);
                    for &mut (ref mut stream, ref log) in hs.iter_mut() {
                        self.write_info_response(is_output, name, stream, log)
                        // let _ = stream.write(msg.as_bytes());
                        // let _ = stream.write(b"\n");
                        // let _ = stream.flush();
                    }
                    hs.clear();
                }
                Ok(())
            // },
            // None => Err(()),
        // }

    }

    pub fn port_exists(&self, is_output: bool, name: &String) -> bool {
        if is_output {
            self.outputs.contains_key(name)
        } else {
            self.inputs.contains_key(name)
        }
    }

    pub fn get_connected(&self, is_output: bool, name: &String) -> Result<HashSet<String>, ()> {
        if !self.port_exists(is_output, name) { return Err(()); }

        if is_output {
            match self.connections.get(name) {
                Some(is) => Ok(is.clone()),
                None => Ok(HashSet::new()),
            }
        } else {
            let mut os = HashSet::new();
            for (o, is) in &self.connections {
                if is.contains(name) {
                    os.insert(o.clone());
                }
            }
            Ok(os)
        }
    }

    pub fn hook(&mut self, h_name: String, name: String, stream: TcpStream, log: slog::Logger) {
        self.mon_hooks
            .entry(h_name)
            .or_insert(HashMap::new())
            .entry(name)
            .or_insert(Vec::new())
            .push((stream, log));
    }

    pub fn connect(&mut self, connecting: bool, oname: &str, iname: &str) -> Result<(), ()> {
        self.connections.connect(connecting, oname, iname);
        if let Some(hs) = self.mon_hooks
                .entry("output_con".to_owned())
                .or_insert(HashMap::new())
                .get_mut(oname) {

            // let msg = format!("{}connection to: {}",
            //                   if connecting {""} else {"dis"}, iname);
            let response = server::Response {
                ret: 0, msg: &format!("{}connection", if connecting {""} else {"dis"}), obj: json!({
                    "output_name": oname,
                    "input_name": iname,
                }) };

            for &mut (ref mut stream, ref log) in hs.iter_mut() {
                server::write_response(log, &response, stream);
            }
            // for stream in hs.iter_mut() {
            //     let _ = stream.write(msg.as_bytes());
            //     let _ = stream.write(b"\n");
            //     let _ = stream.flush();
            // }
            hs.clear();
        }
        if let Some(hs) = self.mon_hooks
                .entry("input_con".to_owned())
                .or_insert(HashMap::new())
                .get_mut(iname) {

            let response = server::Response {
                ret: 0, msg: &format!("{}connection", if connecting {""} else {"dis"}), obj: json!({
                    "output_name": oname,
                    "input_name": iname,
                }) };

            for &mut (ref mut stream, ref log) in hs.iter_mut() {
                server::write_response(log, &response, stream);
            }
            // let msg = format!("{}connection to: {}",
            //                   if connecting {""} else {"dis"}, oname);
            // for stream in hs.iter_mut() {
            //     let _ = stream.write(msg.as_bytes());
            //     let _ = stream.write(b"\n");
            //     let _ = stream.flush();
            // }
            hs.clear();
        }
        Ok(())
    }

    pub fn is_connected(&self, oname: &str, iname: &str) -> bool {
        self.connections.is_connected(oname, iname)
    }

    pub fn set_bal(&mut self, is_output: bool, pname: &String, balance: f32) -> Result<(), ()> {
        if !self.port_exists(is_output, &pname) { return Err(()); }
        // let _bal = serde_json::Number::from_f64(balance as f64).unwrap();
        match is_output {
            true => &mut self.outputs,
            false => &mut self.inputs,
        }.get_mut(pname).unwrap().balance = balance;
        if let Some(mut hs) = self.mon_hooks
                .entry(if is_output {"output_bal".to_owned()} else {"input_bal".to_owned()})
                .or_insert(HashMap::new())
                .remove(pname) {

            for &mut (ref mut stream, ref log) in hs.iter_mut() {
                self.write_info_response(is_output, pname, stream, log)
            }
            // let msg = format!("{}", balance);
            // for stream in hs.iter_mut() {
            //     let _ = stream.write(msg.as_bytes());
            //     let _ = stream.write(b"\n");
            //     let _ = stream.flush();
            // }
            hs.clear();
        }
        Ok(())
    }

    pub fn get_bal(&self, is_output: bool, name: &String) -> Result<f32, ()> {
        match match is_output {
            true => self.outputs.get(name),
            false => self.inputs.get(name),
        } {
            Some(p) => Ok(p.get_balance()),
            None => Err(()),
        }
    }

    pub fn get_bal_pair(&self, is_output: bool, name: &String) -> Result<(f32, f32), ()> {
            match match is_output {
                true => self.outputs.get(name),
                false => self.inputs.get(name),
            } {
                Some(p) => Ok(p.get_balance_pair()),
                None => Err(()),
            }
    }

    pub fn set_mon(&mut self, is_output: bool, name: &String) -> Result<(), ()> {
        if !self.port_exists(is_output, name) { return Err(()); }
        self.monitor.channel = name.to_owned();
        self.monitor.is_input = !is_output;
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    pub connections: HashMap<String, HashSet<String>>,
    pub mixer: MixerConfig,
}

pub fn parse(path: &str, logger: slog::Logger) -> Config {
    let path = std::path::Path::new(path);
    info!(logger, "Parsing config file at path: {:?}", path);

    let mut file = match std::fs::File::open(&path) {
        Err(why) => {
            crit!(logger, "couldn't open {}: {}", path.display(), why.description());
            panic!();
        },
        Ok(file) => file,
    };

    let mut s = String::new();
    if let Err(why) = file.read_to_string(&mut s) {
        crit!(logger, "couldn't read {}: {}", path.display(), why.description());
        panic!();
    }

    let config: Config = serde_json::from_str(&s).log_err(&logger).unwrap();

    return config;
}
