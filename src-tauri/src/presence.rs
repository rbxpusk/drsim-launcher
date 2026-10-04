use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::Duration;

const APP_ID: &str = "1553857740617547886";
const LARGE_IMAGE: &str = "https://deltarunesim.com/assets/rpc/soul.png";
const SITE_URL: &str = "https://deltarunesim.com/";

#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub details: String,
    pub state: String,
    pub start_ms: i64,
}

enum Msg {
    Set(Status),
    Clear,
}

pub struct Presence(Mutex<Sender<Msg>>);

impl Presence {
    pub fn start() -> Self {
        let (tx, rx) = channel::<Msg>();
        std::thread::spawn(move || {
            let mut client: Option<DiscordIpcClient> = None;
            let mut want: Option<Status> = None;
            let mut shown: Option<Status> = None;
            let mut said_offline = false;
            loop {
                match rx.recv_timeout(Duration::from_secs(20)) {
                    Ok(Msg::Set(s)) => want = Some(s),
                    Ok(Msg::Clear) => want = None,
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
                if want == shown && client.is_some() {
                    continue;
                }
                match &want {
                    None => {
                        if let Some(c) = client.as_mut() {
                            let _ = c.clear_activity();
                            let _ = c.close();
                        }
                        client = None;
                        shown = None;
                    }
                    Some(s) => {
                        if client.is_none() {
                            let mut c = DiscordIpcClient::new(APP_ID);
                            match c.connect() {
                                Ok(()) => {
                                    crate::logln!("discord: connected");
                                    client = Some(c);
                                }
                                Err(e) => {
                                    if !said_offline {
                                        crate::logln!("discord: not reachable ({e}); trying again later");
                                        said_offline = true;
                                    }
                                }
                            }
                        }
                        if let Some(c) = client.as_mut() {
                            let mut a = activity::Activity::new().activity_type(activity::ActivityType::Playing).details(s.details.clone());
                            // discord rejects text shorter than 2 chars
                            if s.state.chars().count() >= 2 {
                                a = a.state(s.state.clone());
                            }
                            let a = a
                                .timestamps(activity::Timestamps::new().start(s.start_ms))
                                .assets(activity::Assets::new().large_image(LARGE_IMAGE).large_text("deltarunesim.com"))
                                .buttons(vec![activity::Button::new("Play in browser", SITE_URL)]);
                            if c.set_activity(a).is_ok() {
                                crate::logln!("discord: {} / {}", s.details, s.state);
                                said_offline = false;
                                shown = Some(s.clone());
                            } else {
                                let _ = c.close();
                                client = None;
                                shown = None;
                            }
                        }
                    }
                }
            }
        });
        Presence(Mutex::new(tx))
    }
    pub fn set(&self, s: Status) {
        if let Ok(t) = self.0.lock() {
            let _ = t.send(Msg::Set(s));
        }
    }
    pub fn clear(&self) {
        if let Ok(t) = self.0.lock() {
            let _ = t.send(Msg::Clear);
        }
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}
