extern crate slog;
use std;

pub trait LogError {
    fn log_err(self, &slog::Logger) -> Self;
    fn warn_err(self, &slog::Logger) -> Self;
}

impl<T, E: std::fmt::Debug> LogError for Result<T, E> {
    fn log_err(self, logger: &slog::Logger) -> Self {
        return self.map_err(|expl| { error!(logger, "{:?}", expl); expl });
    }
    fn warn_err(self, logger: &slog::Logger) -> Self {
        return self.map_err(|expl| { warn!(logger, "{:?}", expl); expl });
    }
}

