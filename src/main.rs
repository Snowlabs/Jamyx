extern crate jack;
#[macro_use]
extern crate slog;
extern crate sloggers;

extern crate jam;

use std::io;

use jack::prelude as j;
use sloggers::Build;
use sloggers::terminal::{TerminalLoggerBuilder, Destination};
use sloggers::types::Severity;




fn main() {
    // Init logger
    let mut builder = TerminalLoggerBuilder::new();
    builder.level(Severity::Debug);
    builder.destination(Destination::Stderr);

    let log = builder.build().unwrap();

    info!(log, "Logger init");

    let mut jclient = jam::Client::new("Jacon", log.new(o!()));
    jclient.init(None);

    let cblog = log.new(o!());
    let cb: Box<Fn(&j::Client, &str, bool)+Send> = Box::new(move |c: &j::Client, cn: &str, of: bool| {
        info!(cblog, "Client {}registered: {}", if of {""} else {"un"}, cn)
    });

    jclient.notifications_handler.as_mut().unwrap().hook(jam::CB::client_registration(cb));

    jclient.activate();

    let mut user_input = String::new();
    io::stdin().read_line(&mut user_input).ok();

    jclient.deactivate();
    info!(log, "Jclient deactivate");
}
