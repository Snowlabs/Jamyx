extern crate jack;
#[macro_use]
extern crate slog;
extern crate sloggers;


pub struct Notifications {
    log: slog::Logger,
    hooks: Vec<CB>,
}
impl Notifications {
    pub fn new(log: slog::Logger) -> Notifications {
        Notifications {
            log,
            hooks: Vec::new(),
        }
    }
    pub fn hook(&mut self, cb: CB) {
        self.hooks.push(cb)
    }
}
impl j::NotificationHandler for Notifications {
    fn client_registration(&mut self, cli: &j::Client, name: &str, is_reg: bool) {
        for cb in &self.hooks {
            if let &CB::client_registration(ref c) = cb {
                c(cli, name, is_reg)
            }
        }
    }
}

use jack::prelude as j;

type A = j::AsyncClient<Notifications, ()>;

enum AnyClient {
    Inactive(j::Client),
    Active(A),
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
    ports_connected(Box<Fn(&j::Client, j::JackPortId, j::JackPortId, bool) -> j::JackControl+Send>),

    graph_reorder(Box<Fn(&j::Client) -> j::JackControl+Send>),
    xrun(Box<Fn(&j::Client) -> j::JackControl+Send>),
    latency(Box<Fn(&j::Client, j::LatencyType)+Send>),
}


pub struct Client {
    jclient: Option<AnyClient>,
    name: String,
    logger: slog::Logger,
    hooks: Vec<CB>,
    pub notifications_handler: Option<Notifications>,
}

impl Client {
    pub fn new(name: &str, logger: slog::Logger) -> Client {
        let not_logger = logger.new(o!());
        Client {
            jclient: None,
            name: name.to_string(),
            logger,
            hooks: Vec::new(),
            notifications_handler: Some(Notifications::new(not_logger)),
        }
    }

    pub fn init(&mut self, opts: Option<j::client_options::ClientOptions>) {
        info!(self.logger, "Init");

        // Create client
        let (client, _stat) = j::Client::new(self.name.as_str(), opts.unwrap_or(j::client_options::NO_START_SERVER)).unwrap();
        self.jclient = Some(AnyClient::Inactive(client));

    }

    pub fn activate(&mut self) {
        let not_han = std::mem::replace(&mut self.notifications_handler, None);

        // Activate client
        let inactive_client = std::mem::replace(&mut self.jclient, None);
        if let AnyClient::Inactive(c) = inactive_client.expect("Cannot activate non-initialized client!") {

            self.jclient = Some(AnyClient::Active(j::AsyncClient::new( c, not_han.unwrap(), ()).unwrap()));

        } else {
                panic!("Cannot activate non-inactive client!");
        }

    }

    pub fn deactivate(mut self) -> Self {
        // Deactivate client
        let active_jclient = std::mem::replace(&mut self.jclient, None);

        if let AnyClient::Active(c) = active_jclient.expect("Cannot deactivate non-active client!") {
            c.deactivate().unwrap();
        } else {
            panic!("Cannot activate active client!");
        }

        self
    }

    pub fn hook(&mut self, cb: CB) {
        self.hooks.push(cb)
    }
}


// impl j::NotificationHandler for Client {
//     fn client_registration(&mut self, _: &j::Client, name: &str, is_reg: bool) {
//         for cb in &self.hooks {
//             if let CB::client_registration(c) = cb {
//                 c(&name, is_reg)
//             }
//         }
//     }
// }
