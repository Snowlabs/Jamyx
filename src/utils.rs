extern crate slog;
use std;

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

