extern crate serde;
extern crate serde_json;
extern crate slog;

use std;
use std::error::Error;
use std::io::Read;
use std::collections::{HashMap, HashSet};

use utils::LogError;

#[derive(Serialize, Deserialize, Debug)]
pub struct PortConfig {
    #[serde(default)]
    pub vol: serde_json::Value,
    #[serde(default)]
    pub mono: serde_json::Value,
    #[serde(default)]
    pub balance: serde_json::Value,
}

impl PortConfig {
    pub fn get_vol(&self) -> f32 {
        if self.vol.is_null() {
            100.0 / 100.0
        } else {
            self.vol.as_f64().unwrap() as f32 / 100.0
        }
    }

    pub fn get_balance(&self) -> f32 {
        if self.balance.is_null() {
            0.0
        } else {
            self.balance.as_f64().unwrap() as f32
        }
    }

    pub fn get_balance_pair(&self) -> (f32, f32) {
        let b = self.get_balance();
        (b+1.0, -b+1.0)
    }

    pub fn is_mono(&self) -> bool {
        if self.mono.is_null() {
            false
        } else {
            self.mono.as_bool().unwrap()
        }
    }
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
}

impl MixerConfig {
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

    pub fn port_exists(&self, is_output: bool, name: &String) -> bool {
        if is_output {
            self.outputs.contains_key(name)
        } else {
            self.inputs.contains_key(name)
        }
    }

    pub fn get_connected(&self, is_output: bool, name: &String) -> Result<HashSet<String>, ()> {
        if !self.port_exists(is_output, name) {
            return Err(());
        }
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
