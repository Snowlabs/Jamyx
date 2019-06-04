extern crate serde;
extern crate serde_json;
extern crate slog;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::Sender;
use std::thread;

use std::error::Error;
use utils::LogError;

#[derive(Serialize)]
pub struct Response<'a> {
    pub ret: i32,
    pub msg: &'a str,
    pub obj: serde_json::Value,
}

pub fn write_response(log: &slog::Logger, r: &Response, stream: &mut TcpStream) {
    let msg = serde_json::to_string(&r).unwrap();
    // let msg = format!("Bad command: `{}`", command.cmd);
    let _ = stream.write(msg.as_bytes());
    let _ = stream.write(b"\n");
    let _ = stream.flush().log_err(&log);
    debug!(log, "{}", msg);
}

#[derive(Serialize, Deserialize)]
pub struct Command {
    pub target: String,
    pub cmd: String,
    pub opts: Vec<String>,
}

pub struct CmdSender {
    myx: Sender<(TcpStream, Command)>,
    con: Sender<(TcpStream, Command)>,
    all: Sender<(TcpStream, Command)>,
}
impl CmdSender {
    pub fn new(
        myx: Sender<(TcpStream, Command)>,
        con: Sender<(TcpStream, Command)>,
        all: Sender<(TcpStream, Command)>,
    ) -> Self {
        CmdSender { myx, con, all }
    }
    pub fn send(&self, s: TcpStream, c: Command) {
        match c.target.as_str() {
            "myx" => {
                self.myx.send((s, c)).unwrap();
            }
            "con" => {
                self.con.send((s, c)).unwrap();
            }
            "all" => {
                self.all.send((s, c)).unwrap();
            }
            _ => {}
        }
    }
}
impl Clone for CmdSender {
    fn clone(&self) -> Self {
        CmdSender {
            myx: self.myx.clone(),
            con: self.con.clone(),
            all: self.all.clone(),
        }
    }
}

fn handle_client(log: slog::Logger, mut stream: TcpStream, sender: CmdSender) {
    let peer_addr = stream.peer_addr().unwrap();
    info!(log, "New connection from: {}", peer_addr);

    loop {
        let mut ibuff = [0; 256];
        // let mut obuff = [0; 256];
        let _ = stream.read(&mut ibuff).log_err(&log);

        let mut ibuff = ibuff.to_vec();
        ibuff.retain(|&b| b != 0);
        let ibuff = String::from_utf8_lossy(ibuff.as_slice());

        debug!(log, "RECVD: {}", ibuff);
        let cmd: Result<Command, serde_json::error::Error> = 
            serde_json::from_str(&ibuff);

        match cmd {
            Ok(cmd) => {
                sender.send(stream.try_clone().unwrap(), cmd);
                return;
            }
            Err(e) => {
                error!(
                    log,
                    "{}: {:?} ({} : {})",
                    e.description(),
                    e.classify(),
                    e.line(),
                    e.column()
                );
                let _ = stream.write(e.description().as_bytes());
                return;
            }
        }
    }
}

pub fn start(log: slog::Logger, sender: CmdSender) {
    let listener = TcpListener::bind("127.0.0.1:56065").unwrap();

    // accept and handle incoming connections
    thread::spawn(move || {
        for stream in listener.incoming() {
            let log = log.clone();
            let sender = sender.clone();
            let _ = stream
                .log_err(&log)
                .map(move |s| {
                    thread::spawn(move || {
                        handle_client(
                            log.new(o!("peer address" => format!("{}", s.peer_addr().unwrap()))),
                            s,
                            sender,
                        );
                    });
                });
        }
    });
}
