extern crate jack;
#[macro_use]
extern crate slog;
extern crate sloggers;

use std::sync::{Arc, Mutex};
use std::sync::atomic::AtomicBool;
use std::mem;
use std::thread;

#[derive(Debug)]
pub enum JamyxErr {
    InitError(String),
    ActivateError(String),
    DeactivateError(String),

    ClientIsActive,
    ClientIsInactive,
    ClientIsNone,

    Other(String)
}

trait JerrDerive {}
impl<'a> JerrDerive for &'a str {}
impl JerrDerive for j::JackErr {}
impl<T> JerrDerive for std::sync::PoisonError<T> {}

impl<T: std::fmt::Debug + JerrDerive > From<T> for JamyxErr {
    fn from(e: T) -> JamyxErr {
        JamyxErr::Other(format!("{:?}", e))
    }
}


macro_rules! def_cb {
    ($name:ident) => (
        fn $name(&mut self) {
            for cb in &*self.hooks.lock().unwrap() {
                if let &CB::$name(ref c) = cb {
                    c();
                }
            }
        }
    );
    ($name:ident, $($arg:ident: $type:ty),+) => (
        fn $name(&mut self, $($arg: $type),+) {
            for cb in &*self.hooks.lock().unwrap() {
                if let &CB::$name(ref c) = cb {
                    c($($arg),+);
                }
            }
        }
    );
}
macro_rules! def_cb_immut {
    ($name:ident, $($arg:ident: $type:ty),+) => (
        fn $name(&self, $($arg: $type),+) {
            for cb in &*self.hooks.lock().unwrap() {
                if let &CB::$name(ref c) = cb {
                    c($($arg),+);
                }
            }
        }
    )
}
macro_rules! def_cb_ret {
    ($name:ident, $($arg:ident: $type:ty),+) => (
        fn $name(&mut self, $($arg: $type),+) -> j::JackControl {
            for cb in &*self.hooks.lock().unwrap() {
                if let &CB::$name(ref c) = cb {
                    let ret = c($($arg),+);
                    if ret == j::JackControl::Quit {
                        return ret;
                    }
                }
            }
            return j::JackControl::Continue;
        }
    )
}

pub struct Notifications {
    log: slog::Logger,
    hooks: Arc<Mutex<Vec<CB>>>,
}
unsafe impl Send for Notifications { }
impl Notifications {
    pub fn new(log: slog::Logger, hooks: Arc<Mutex<Vec<CB>>>) -> Notifications {
        Notifications {
            log,
            hooks,
        }
    }
    pub fn hook(&mut self, cb: CB) {
        self.hooks.lock().unwrap().push(cb)
    }

    // def_cb!(client_reconnect);
}


impl j::NotificationHandler for Notifications {
    def_cb_immut!(thread_init, cli: &j::Client);
    def_cb!(shutdown, cli: j::ClientStatus, s: &str);
    def_cb!(freewheel, cli: &j::Client, of: bool);
    def_cb_ret!(buffer_size, cli: &j::Client, f: j::JackFrames);
    def_cb_ret!(sample_rate, cli: &j::Client, f: j::JackFrames);

    def_cb!(client_registration, cli: &j::Client, s: &str, of: bool);
    def_cb!(port_registration, cli: &j::Client, id: j::JackPortId, of: bool);
    def_cb_ret!(port_rename, cli: &j::Client, id: j::JackPortId, s: &str, s2: &str);
    def_cb!(ports_connected, cli: &j::Client, id: j::JackPortId, id2: j::JackPortId, of: bool);

    def_cb_ret!(graph_reorder, cli: &j::Client);
    def_cb_ret!(xrun, cli: &j::Client);
    def_cb!(latency, cli: &j::Client, lat: j::LatencyType);
}

use jack::prelude as j;

type A = j::AsyncClient<Notifications, ()>;

pub enum AnyClient {
    None,
    Inactive(j::Client),
    Active(A),
}

impl AnyClient {
    pub fn as_inactive(&self) -> Result<&j::Client, JamyxErr> {
        match *self {
            AnyClient::Inactive(ref c) => Ok(c),
            AnyClient::Active(ref c) => Ok(c),
            AnyClient::None => Err(JamyxErr::ClientIsNone)
        }
    }

    pub fn to_inactive(&mut self) -> Result<j::Client, JamyxErr> {
        let cli = std::mem::replace(&mut *self, AnyClient::None);
        match cli {
            AnyClient::Inactive(c) => Ok(c),
            AnyClient::Active(c) => Err(JamyxErr::ClientIsActive),
            AnyClient::None => Err(JamyxErr::ClientIsNone)
        }
    }

    pub fn as_active(&self) -> Result<&A, JamyxErr> {
        match *self {
            AnyClient::Active(ref c) => Ok(c),
            AnyClient::Inactive(_) => Err(JamyxErr::ClientIsInactive),
            AnyClient::None => Err(JamyxErr::ClientIsNone)
        }
    }

    pub fn to_active(&mut self) -> Result<A, JamyxErr> {
        let cli = std::mem::replace(&mut *self, AnyClient::None);
        match cli {
            AnyClient::Active(c) => Ok(c),
            AnyClient::Inactive(_) => Err(JamyxErr::ClientIsInactive),
            AnyClient::None => Err(JamyxErr::ClientIsNone)
        }
    }
}

pub trait JackClientUtils {
    fn connect_ports_by_name_if(&self, &bool, &str, &str) -> Result<(), j::JackErr>;
    fn port_name_by_id(&self, j::JackPortId) -> Option<String>;
}

impl JackClientUtils for j::Client {
    fn connect_ports_by_name_if(&self, connecting: &bool, iname: &str, oname: &str) -> Result<(), j::JackErr>{
        if *connecting {
            self.connect_ports_by_name(&iname, &oname)
        } else {
            self.disconnect_ports_by_name(&iname, &oname)
        }
    }

    fn port_name_by_id(&self, port_id: j::JackPortId) -> Option<String> {
        match self.port_by_id(port_id) {
            Some(p) => Some(p.name().to_string()),
            None => None
        }
    }
}

pub enum CB {
    thread_init(Box<Fn(&j::Client)+Send>),
    shutdown(Box<Fn(j::ClientStatus, &str)+Send>),
    freewheel(Box<Fn(&j::Client, bool)+Send>),
    buffer_size(Box<Fn(&j::Client, j::JackFrames) -> j::JackControl+Send>),
    sample_rate(Box<Fn(&j::Client, j::JackFrames) -> j::JackControl+Send>),

    client_registration(Box<Fn(&j::Client, &str, bool)+Send>),
    port_registration(Box<Fn(&j::Client, j::JackPortId, bool)+Send>),
    port_rename(Box<Fn(&j::Client, j::JackPortId, &str, &str) -> j::JackControl+Send>),
    ports_connected(Box<Fn(&j::Client, j::JackPortId, j::JackPortId, bool)+Send>),

    graph_reorder(Box<Fn(&j::Client) -> j::JackControl+Send>),
    xrun(Box<Fn(&j::Client) -> j::JackControl+Send>),
    latency(Box<Fn(&j::Client, j::LatencyType)+Send>),

    client_reconnection(Box<Fn()+Send>),
}


pub struct Client {
    pub jclient: Arc<Mutex<AnyClient>>,
    name: String,
    logger: slog::Logger,
    pub notifications_handler: Arc<Mutex<Option<Notifications>>>,
    do_recon: Arc<Mutex<bool>>,
    hooks: Arc<Mutex<Vec<CB>>>,
}

impl Client {
    pub fn new(name: &str, logger: slog::Logger) -> Client {
        let not_logger = logger.new(o!());
        let hooks = Arc::new(Mutex::new(Vec::new()));
        Client {
            jclient: Arc::new(Mutex::new(AnyClient::None)),
            name: name.to_string(),
            logger,
            notifications_handler: Arc::new(Mutex::new(Some(Notifications::new(not_logger, hooks.clone())))),
            do_recon: Arc::new(Mutex::new(false)),
            hooks: hooks.clone(),
        }
    }

    pub fn init(&mut self, opts: Option<j::client_options::ClientOptions>) -> Result<(), JamyxErr> {
        info!(self.logger, "Init");

        // Create client
        let (client, _stat) = j::Client::new(self.name.as_str(), opts.unwrap_or(j::client_options::NO_START_SERVER))?;
        let mut jcli = self.jclient.lock()?;
        *jcli = AnyClient::Inactive(client);

        let logger = self.logger.clone();
        let do_recon = self.do_recon.clone();
        let jcli = self.jclient.clone();
        let not_han = self.notifications_handler.clone();
        let hooks = self.hooks.clone();
        self.notifications_handler.lock().unwrap().as_mut().unwrap().hook(CB::shutdown(Box::new(move |cs, s| {
            warn!(logger, "Server shutdown! code: {:?}, reason: {}", cs, s);
            *do_recon.lock().unwrap() = true;

            let mut old_cli = mem::replace(&mut *jcli.lock().unwrap(), AnyClient::None).to_active().unwrap();
            unsafe { mem::forget(old_cli); }
            // Still "recover" the notifications_handler
            *not_han.lock().unwrap() = Some(Notifications::new(logger.clone(), hooks.clone()));
        })));

        Ok(())
    }

    pub fn activate(&mut self) -> Result<(), JamyxErr> {
        // Activate client
        let mut jcli = self.jclient.lock()?;

        match &*jcli {
            &AnyClient::Inactive(_) => {
                let inactive_client = mem::replace(&mut *jcli, AnyClient::None).to_inactive()?;
                let not_han = mem::replace(&mut *self.notifications_handler.lock()?, None);

                *jcli = AnyClient::Active(j::AsyncClient::new(inactive_client, not_han.unwrap(), ())?);
                Ok(())
            }
            &AnyClient::Active(_) => Err(JamyxErr::ActivateError("Cannot activate already activated client!".to_string())),
            &AnyClient::None => Err(JamyxErr::ActivateError("Cannot activate non-initialized client!".to_string()))
        }
    }

    pub fn start_reconnection_loop(&mut self) -> Result<(), JamyxErr> {
        *self.do_recon.lock().unwrap() = false;

        let do_recon = self.do_recon.clone();
        let jcli = self.jclient.clone();
        let logger = self.logger.clone();
        let not_han = self.notifications_handler.clone();
        let hooks = self.hooks.clone();
        thread::spawn(move || {
            loop {
                while *do_recon.lock().unwrap() {
                    debug!(logger, "===== Attempting reconnection... =====");
                    let c_res = j::Client::new("RESURRECT", (j::client_options::NO_START_SERVER));
                    debug!(logger, "===== ... =====");
                    match c_res {
                        Ok((client, _stat)) => {
                            // let (client, _stat) = j::Client::new("RESURRECT", (j::client_options::NO_START_SERVER)).unwrap();
                            debug!(logger, "STATUS OF TRY: `{:?}`", _stat);
                            let mut jcli = jcli.lock().unwrap();
                            let not_han = mem::replace(&mut *not_han.lock().unwrap(), None);
                            *jcli = AnyClient::Active(j::AsyncClient::new(client, not_han.unwrap(), ()).unwrap());

                            *do_recon.lock().unwrap() = false;
                            for h in &*hooks.lock().unwrap() {
                                if let &CB::client_reconnection(ref cb) = h { cb(); }
                            }
                        }
                        Err(e) => {
                            warn!(logger, "Failed to open client because of error: {:?}", e);
                            thread::sleep(std::time::Duration::from_millis(2000));
                        }
                    }

                }
                thread::sleep(std::time::Duration::from_millis(1000))
            }
        });
        Ok(())
    }

    pub fn deactivate(&mut self) -> Result<(), JamyxErr>{
        // Deactivate client
        let mut jcli = self.jclient.lock()?;

        match &*jcli {
            &AnyClient::Active(_) => {
                let active_client = mem::replace(&mut *jcli, AnyClient::None).to_active()?;

                let (_jcli, not_han, proc_han) = active_client.deactivate()?;
                *jcli = AnyClient::Inactive(_jcli);
                *self.notifications_handler.lock().unwrap() = Some(not_han);
                Ok(())
            }
            &AnyClient::Inactive(_) => Err(JamyxErr::DeactivateError("Cannot deactivate already inactive client!".to_string())),
            &AnyClient::None => Err(JamyxErr::DeactivateError("Cannot deactivate non-initialized client!".to_string()))
        }
    }

    pub fn hook(&mut self, cb: CB) {
        self.notifications_handler.lock().unwrap().as_mut().unwrap().hook(cb);
    }

}
