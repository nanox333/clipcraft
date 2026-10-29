//! Wayland global shortcuts through the desktop portal. No keyboard devices are read unless the
//! user explicitly enabled that fallback. Portal setup and its chooser run off the UI thread.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU32, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

use async_io::Timer;
use clipcraft_capture::CaptureError;
use futures_lite::{StreamExt, future};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, MatchRule, MessageStream, Proxy, message::Type};

use crate::{HotkeyBinding, HotkeyEvent, HotkeyListener, HotkeyStatus, parse_shortcut};

const DESKTOP: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const GLOBAL: &str = "org.freedesktop.portal.GlobalShortcuts";
const REQUEST: &str = "org.freedesktop.portal.Request";
const SESSION: &str = "org.freedesktop.portal.Session";
const MAX_BINDINGS: usize = 25;
const WAIT: Duration = Duration::from_secs(180);
const POLL: Duration = Duration::from_millis(100);
type Results = HashMap<String, OwnedValue>;
type BoundShortcuts = Vec<(String, HashMap<String, OwnedValue>)>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Failure {
    Unavailable,
    Denied,
    Cancelled,
}

fn token(kind: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!(
        "clipcraft_{kind}_{}_{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// Convert ClipCraft's canonical shortcut to the XDG shortcut specification's XKB names.
fn preferred_trigger(shortcut: &str) -> Option<String> {
    let (modifiers, name) = parse_shortcut(shortcut)?;
    let key = match name.as_str() {
        "ArrowDown" => "Down",
        "ArrowLeft" => "Left",
        "ArrowRight" => "Right",
        "ArrowUp" => "Up",
        "Backspace" => "BackSpace",
        "Enter" => "Return",
        "PageDown" => "Page_Down",
        "PageUp" => "Page_Up",
        "Space" => "space",
        "Minus" => "minus",
        "Equals" => "equal",
        "Comma" => "comma",
        "Period" => "period",
        "Slash" => "slash",
        "Backtick" => "grave",
        name if name.len() == 1 && name.as_bytes()[0].is_ascii_alphabetic() => {
            // XKB base-layer letter keysyms are lowercase.
            return Some(format!(
                "{}{}{}{}",
                if modifiers.ctrl { "CTRL+" } else { "" },
                if modifiers.alt { "ALT+" } else { "" },
                if modifiers.shift { "SHIFT+" } else { "" },
                name.to_ascii_lowercase()
            ));
        }
        name if name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') => name,
        _ => return None,
    };
    Some(format!(
        "{}{}{}{}",
        if modifiers.ctrl { "CTRL+" } else { "" },
        if modifiers.alt { "ALT+" } else { "" },
        if modifiers.shift { "SHIFT+" } else { "" },
        key
    ))
}

async fn interruptible<T, E>(
    future: impl Future<Output = Result<T, E>>,
    stop: &AtomicBool,
    timeout: Duration,
) -> Result<T, Failure> {
    let deadline = Instant::now() + timeout;
    future::race(
        async { future.await.map_err(|_| Failure::Unavailable) },
        async {
            loop {
                if stop.load(Ordering::Acquire) {
                    return Err(Failure::Cancelled);
                }
                if Instant::now() >= deadline {
                    return Err(Failure::Unavailable);
                }
                Timer::after(POLL).await;
            }
        },
    )
    .await
}

async fn stream(
    connection: &Connection,
    sender: &str,
    interface: &str,
    path: &str,
    member: Option<&str>,
    stop: &AtomicBool,
) -> Result<MessageStream, Failure> {
    let mut builder = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(sender)
        .map_err(|_| Failure::Unavailable)?
        .interface(interface)
        .map_err(|_| Failure::Unavailable)?
        .path(path)
        .map_err(|_| Failure::Unavailable)?;
    if let Some(member) = member {
        builder = builder.member(member).map_err(|_| Failure::Unavailable)?;
    }
    interruptible(
        MessageStream::for_match_rule(builder.build(), connection, Some(64)),
        stop,
        Duration::from_secs(5),
    )
    .await
}

async fn watch_owner(connection: &Connection, stop: &AtomicBool) -> Result<MessageStream, Failure> {
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(|_| Failure::Unavailable)?
        .interface("org.freedesktop.DBus")
        .map_err(|_| Failure::Unavailable)?
        .member("NameOwnerChanged")
        .map_err(|_| Failure::Unavailable)?
        .add_arg(DESKTOP)
        .map_err(|_| Failure::Unavailable)?
        .build();
    interruptible(
        MessageStream::for_match_rule(rule, connection, Some(4)),
        stop,
        Duration::from_secs(5),
    )
    .await
}

async fn request<T: Future<Output = Result<zbus::Message, zbus::Error>>>(
    connection: &Connection,
    owner: &str,
    token: &str,
    method: T,
    stop: &AtomicBool,
) -> Result<Results, Failure> {
    let sender = connection
        .unique_name()
        .ok_or(Failure::Unavailable)?
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    let mut responses = stream(connection, owner, REQUEST, &path, Some("Response"), stop).await?;
    let mut owners = watch_owner(connection, stop).await?;
    let response = interruptible(method, stop, Duration::from_secs(10)).await?;
    let returned = response
        .body()
        .deserialize::<OwnedObjectPath>()
        .map_err(|_| Failure::Unavailable)?;
    if returned.as_str() != path {
        return Err(Failure::Unavailable);
    }
    let signal = interruptible(
        future::race(
            async {
                responses
                    .next()
                    .await
                    .ok_or(zbus::Error::Failure("portal closed".into()))?
            },
            async {
                let _ = owners.next().await;
                Err(zbus::Error::Failure("portal owner changed".into()))
            },
        ),
        stop,
        WAIT,
    )
    .await;
    if signal.is_err() {
        // Close the desktop's pending chooser when bindings are changed or the app exits.
        if let Ok(proxy) = Proxy::new(connection, owner, path.as_str(), REQUEST).await {
            let _ = interruptible(
                proxy.call_method("Close", &()),
                &AtomicBool::new(false),
                Duration::from_secs(1),
            )
            .await;
        }
    }
    let signal = signal?;
    if signal.data().bytes().len() > 65_536 {
        return Err(Failure::Unavailable);
    }
    let (code, results): (u32, Results) = signal
        .body()
        .deserialize()
        .map_err(|_| Failure::Unavailable)?;
    match code {
        0 => Ok(results),
        1 => Err(Failure::Denied),
        _ => Err(Failure::Unavailable),
    }
}

fn set_status(status: &Mutex<HotkeyStatus>, value: HotkeyStatus) {
    *status
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = value;
}

fn send_event(sender: &mpsc::SyncSender<HotkeyEvent>, overflow: &AtomicBool, event: HotkeyEvent) {
    if sender.try_send(event).is_err() {
        overflow.store(true, Ordering::Release);
    }
}

fn accepted_shortcuts(
    bindings: &[HotkeyBinding],
    bound: BoundShortcuts,
) -> (HashSet<u32>, HotkeyStatus) {
    let mut valid = HashSet::new();
    let mut assigned = Vec::new();
    for (id, properties) in bound.into_iter().take(MAX_BINDINGS) {
        let Some(binding) = bindings
            .iter()
            .find(|binding| id == format!("shortcut_{}", binding.id))
        else {
            continue;
        };
        if !valid.insert(binding.id) {
            continue;
        }
        let description = properties
            .get("trigger_description")
            .and_then(|value| value.try_clone().ok())
            .and_then(|value| String::try_from(value).ok())
            .filter(|value| {
                !value.is_empty() && value.len() <= 160 && !value.chars().any(char::is_control)
            })
            .unwrap_or_else(|| "Set in desktop settings".to_owned());
        assigned.push((binding.shortcut.clone(), description));
    }
    let unbound = bindings
        .iter()
        .filter(|binding| !valid.contains(&binding.id))
        .map(|binding| binding.shortcut.clone())
        .collect();
    (valid, HotkeyStatus::Portal { unbound, assigned })
}

async fn portal(
    bindings: &[HotkeyBinding],
    address: Option<&str>,
    stop: &AtomicBool,
    sender: &mpsc::SyncSender<HotkeyEvent>,
    overflow: &AtomicBool,
    status: &Mutex<HotkeyStatus>,
) -> Result<(), Failure> {
    let connection = if let Some(address) = address {
        let builder =
            zbus::connection::Builder::address(address).map_err(|_| Failure::Unavailable)?;
        interruptible(builder.build(), stop, Duration::from_secs(5)).await?
    } else {
        interruptible(Connection::session(), stop, Duration::from_secs(5)).await?
    };
    let proxy = interruptible(
        Proxy::new(&connection, DESKTOP, PATH, GLOBAL),
        stop,
        Duration::from_secs(5),
    )
    .await?;
    let version: u32 =
        interruptible(proxy.get_property("version"), stop, Duration::from_secs(5)).await?;
    if version == 0 {
        return Err(Failure::Unavailable);
    }
    let bus = interruptible(
        zbus::fdo::DBusProxy::new(&connection),
        stop,
        Duration::from_secs(5),
    )
    .await?;
    let name = zbus::names::BusName::try_from(DESKTOP).map_err(|_| Failure::Unavailable)?;
    let owner = interruptible(bus.get_name_owner(name), stop, Duration::from_secs(5))
        .await?
        .to_string();
    let create_token = token("create");
    let session_token = token("session");
    let create_options = HashMap::from([
        ("handle_token", Value::from(create_token.as_str())),
        ("session_handle_token", Value::from(session_token.as_str())),
    ]);
    let created = request(
        &connection,
        &owner,
        &create_token,
        proxy.call_method("CreateSession", &(create_options,)),
        stop,
    )
    .await?;
    let session = created
        .get("session_handle")
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| String::try_from(v).ok())
        .ok_or(Failure::Unavailable)?;
    let session_path = ObjectPath::try_from(session.as_str()).map_err(|_| Failure::Unavailable)?;
    let result = async {
        let mut events = stream(&connection, &owner, GLOBAL, PATH, None, stop).await?;
        let mut closed =
            stream(&connection, &owner, SESSION, &session, Some("Closed"), stop).await?;
        let mut owner_changes = watch_owner(&connection, stop).await?;
        let mut shortcuts = Vec::new();
        for binding in bindings {
            let Some(trigger) = preferred_trigger(&binding.shortcut) else {
                continue;
            };
            let label = clipcraft_engine::Hotkeys::action_for_id(binding.id)
                .map_or("ClipCraft shortcut", |action| action.label());
            shortcuts.push((
                format!("shortcut_{}", binding.id),
                HashMap::from([
                    ("description", Value::from(label)),
                    ("preferred_trigger", Value::from(trigger)),
                ]),
            ));
        }
        let bind_token = token("bind");
        let options = HashMap::from([("handle_token", Value::from(bind_token.as_str()))]);
        let bound = request(
            &connection,
            &owner,
            &bind_token,
            proxy.call_method("BindShortcuts", &(&session_path, &shortcuts, "", options)),
            stop,
        )
        .await;
        match bound {
            Ok(bound) => {
                let bound: BoundShortcuts = bound
                    .get("shortcuts")
                    .and_then(|v| v.try_clone().ok())
                    .and_then(|v| Vec::try_from(v).ok())
                    .ok_or(Failure::Unavailable)?;
                let (mut valid, initial_status) = accepted_shortcuts(bindings, bound);
                if valid.is_empty() {
                    return Err(Failure::Unavailable);
                }
                set_status(status, initial_status);
                loop {
                    if stop.load(Ordering::Acquire) {
                        break Err(Failure::Cancelled);
                    }
                    enum Next {
                        Event(Option<Result<zbus::Message, zbus::Error>>),
                        Closed,
                        OwnerChanged,
                        Tick,
                    }
                    let next = future::race(
                        async { Next::Event(events.next().await) },
                        future::race(
                            async {
                                let _ = closed.next().await;
                                Next::Closed
                            },
                            future::race(
                                async {
                                    let _ = owner_changes.next().await;
                                    Next::OwnerChanged
                                },
                                async {
                                    Timer::after(POLL).await;
                                    Next::Tick
                                },
                            ),
                        ),
                    )
                    .await;
                    match next {
                        Next::Tick => {}
                        Next::Closed
                        | Next::OwnerChanged
                        | Next::Event(None)
                        | Next::Event(Some(Err(_))) => {
                            break Err(Failure::Unavailable);
                        }
                        Next::Event(Some(Ok(message))) => {
                            if message.data().bytes().len() > 65_536 {
                                continue;
                            }
                            let member = message.header().member().map(|m| m.as_str().to_owned());
                            if member.as_deref() == Some("ShortcutsChanged") {
                                if let Ok((received, changed)) =
                                    message
                                        .body()
                                        .deserialize::<(OwnedObjectPath, BoundShortcuts)>()
                                    && received.as_str() == session
                                {
                                    let (updated, updated_status) =
                                        accepted_shortcuts(bindings, changed);
                                    if updated.is_empty() {
                                        break Err(Failure::Unavailable);
                                    }
                                    valid = updated;
                                    overflow.store(true, Ordering::Release);
                                    set_status(status, updated_status);
                                }
                                continue;
                            }
                            let pressed = match member.as_deref() {
                                Some("Activated") => true,
                                Some("Deactivated") => false,
                                _ => continue,
                            };
                            let Ok((received, id, _, _)) =
                                message
                                    .body()
                                    .deserialize::<(OwnedObjectPath, String, u64, Results)>()
                            else {
                                continue;
                            };
                            if received.as_str() != session {
                                continue;
                            }
                            let Some(id) = id
                                .strip_prefix("shortcut_")
                                .and_then(|s| s.parse::<u32>().ok())
                                .filter(|id| valid.contains(id))
                            else {
                                continue;
                            };
                            send_event(sender, overflow, HotkeyEvent { id, pressed });
                        }
                    }
                }
            }
            Err(error) => Err(error),
        }
    }
    .await;
    if let Ok(session_proxy) = Proxy::new(&connection, DESKTOP, session_path, SESSION).await {
        let _ = interruptible(
            session_proxy.call_method("Close", &()),
            &AtomicBool::new(false),
            Duration::from_secs(1),
        )
        .await;
    }
    result
}

pub(crate) fn start(
    bindings: Vec<HotkeyBinding>,
    allow_raw: bool,
    address: Option<String>,
) -> Result<HotkeyListener, CaptureError> {
    let mut ids = HashSet::new();
    if bindings.len() > MAX_BINDINGS
        || bindings.iter().any(|binding| {
            !ids.insert(binding.id)
                || clipcraft_engine::Hotkeys::action_for_id(binding.id).is_none()
                || clipcraft_engine::normalize_hotkey(&binding.shortcut).as_deref()
                    != Some(binding.shortcut.as_str())
        })
    {
        return Err(CaptureError::Failed);
    }
    let (sender, receiver) = mpsc::sync_channel(64);
    let stop = Arc::new(AtomicBool::new(false));
    let overflow = Arc::new(AtomicBool::new(false));
    let status = Arc::new(Mutex::new(HotkeyStatus::Pending));
    let (worker_stop, worker_overflow, worker_status) =
        (stop.clone(), overflow.clone(), status.clone());
    let worker = std::thread::Builder::new()
        .name("clipcraft-global-shortcuts".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                future::block_on(portal(
                    &bindings,
                    address.as_deref(),
                    &worker_stop,
                    &sender,
                    &worker_overflow,
                    &worker_status,
                ))
            }))
            .unwrap_or(Err(Failure::Unavailable));
            if worker_stop.load(Ordering::Acquire) {
                return;
            }
            // Release held keys only if the portal had become active before it disappeared.
            if matches!(
                *worker_status
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
                HotkeyStatus::Portal { .. }
            ) {
                worker_overflow.store(true, Ordering::Release);
            }
            let denied = result == Err(Failure::Denied);
            let fallback = if allow_raw && address.is_none() {
                crate::evdev::start(bindings.clone())
                    .ok()
                    .map(|v| (v, true))
            } else {
                None
            }
            .or_else(|| {
                address
                    .is_none()
                    .then(|| crate::x11::start_hotkeys(None, bindings))
                    .and_then(Result::ok)
                    .map(|v| (v, false))
            });
            if let Some((listener, direct_keyboard)) = fallback {
                set_status(
                    &worker_status,
                    HotkeyStatus::Fallback {
                        direct_keyboard,
                        denied,
                        unbound: listener.skipped().to_vec(),
                    },
                );
                while !worker_stop.load(Ordering::Acquire) {
                    if listener.is_stopped() {
                        worker_overflow.store(true, Ordering::Release);
                        set_status(&worker_status, HotkeyStatus::Unavailable { denied: false });
                        break;
                    }
                    while let Some(event) = listener.try_next() {
                        send_event(&sender, &worker_overflow, event);
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                listener.request_stop();
                let deadline = Instant::now() + Duration::from_secs(2);
                while !listener.is_stopped() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
            } else {
                set_status(&worker_status, HotkeyStatus::Unavailable { denied });
            }
        })
        .map_err(|_| CaptureError::Failed)?;
    let mut listener = HotkeyListener::from_channel(receiver, stop, Vec::new(), worker);
    listener.portal_status = Some(status);
    listener.overflow = Some(overflow);
    Ok(listener)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferred_keys_follow_xdg_names() {
        assert_eq!(
            preferred_trigger("Ctrl+Shift+A").as_deref(),
            Some("CTRL+SHIFT+a")
        );
        assert_eq!(
            preferred_trigger("Alt+Enter").as_deref(),
            Some("ALT+Return")
        );
        assert_eq!(preferred_trigger("F8").as_deref(), Some("F8"));
        assert_eq!(preferred_trigger("ArrowLeft").as_deref(), Some("Left"));
    }
}
