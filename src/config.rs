extern crate serde;
extern crate serde_json;
extern crate slog;

use std;
use std::error::Error;
use std::io::Read;
use std::collections::HashMap;

#[derive(Serialize, Deserialize)]
pub struct Config {
    pub connections: HashMap<String, Vec<String>>
}

pub fn parse(path: &str, _: slog::Logger) -> Config {
    let path = std::path::Path::new(path);

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
