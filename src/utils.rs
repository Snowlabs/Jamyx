extern crate slog;
use std;

use std::collections::{HashMap, HashSet};

pub trait LogError<T> {
    fn log_err(self, &slog::Logger) -> Self;
    fn warn_err(self, &slog::Logger) -> Self;

    fn log_expect(self, &slog::Logger) -> T;

    fn consume(self);
}

impl<T, E: std::fmt::Debug> LogError<T> for Result<T, E> {
    fn log_err(self, logger: &slog::Logger) -> Self {
        return self.map_err(|expl| { error!(logger, "{:?}", expl); expl });
    }
    fn warn_err(self, logger: &slog::Logger) -> Self {
        return self.map_err(|expl| { warn!(logger, "{:?}", expl); expl });
    }

    fn log_expect(self, logger: &slog::Logger) -> T {
        match self {
            Ok(o) => o,
            Err(e) => {
                crit!(logger, "{:?}", e);
                panic!();
            },
        }
    }

    fn consume(self) {}
}

pub trait Connections {
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
