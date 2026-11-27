//! The window manager, through EWMH on X11 (and XWayland): which window is in front, the
//! windows that are open, bringing one forward, minimizing, closing, keeping Sofa above, and
//! the keys of the YouTube remote (XTest). Every EWMH window manager (Mutter, KWin, Xfwm,
//! Openbox, i3...) honours these from a pager, which is what Sofa is to them.
//!
//! Under Wayland only X11 windows are seen this way: Steam, Proton games and most SDL games
//! run through XWayland, apps written for Wayland do not (PRODUCT.md, "Linux").

use std::sync::{Mutex, MutexGuard, OnceLock};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

/// EWMH's source indication for pagers and other tools acting for the person.
const FROM_TOOL: u32 = 2;

struct Atoms {
    active: Atom,
    clients: Atom,
    stacking: Atom,
    pid: Atom,
    name: Atom,
    utf8: Atom,
    state: Atom,
    fullscreen: Atom,
    above: Atom,
    close: Atom,
    change_state: Atom,
    kind: Atom,
    desktop: Atom,
    dock: Atom,
    icon: Atom,
}

fn atom(c: &RustConnection, name: &str) -> Atom {
    c.intern_atom(false, name.as_bytes()).ok().and_then(|r| r.reply().ok()).map_or(0, |r| r.atom)
}

impl Atoms {
    fn new(c: &RustConnection) -> Atoms {
        Atoms {
            active: atom(c, "_NET_ACTIVE_WINDOW"),
            clients: atom(c, "_NET_CLIENT_LIST"),
            stacking: atom(c, "_NET_CLIENT_LIST_STACKING"),
            pid: atom(c, "_NET_WM_PID"),
            name: atom(c, "_NET_WM_NAME"),
            utf8: atom(c, "UTF8_STRING"),
            state: atom(c, "_NET_WM_STATE"),
            fullscreen: atom(c, "_NET_WM_STATE_FULLSCREEN"),
            above: atom(c, "_NET_WM_STATE_ABOVE"),
            close: atom(c, "_NET_CLOSE_WINDOW"),
            change_state: atom(c, "WM_CHANGE_STATE"),
            kind: atom(c, "_NET_WM_WINDOW_TYPE"),
            desktop: atom(c, "_NET_WM_WINDOW_TYPE_DESKTOP"),
            dock: atom(c, "_NET_WM_WINDOW_TYPE_DOCK"),
            icon: atom(c, "_NET_WM_ICON"),
        }
    }
}

struct X {
    c: RustConnection,
    root: Window,
    a: Atoms,
    /// The keyboard map, for XTest: first keycode, keysyms per keycode, keysyms.
    keys: Option<(u8, u8, Vec<u32>)>,
}

static X: OnceLock<Option<Mutex<X>>> = OnceLock::new();

/// The windows the window manager lists, as the watch thread last saw them (None: not watched).
static CLIENTS: Mutex<Option<Vec<u32>>> = Mutex::new(None);

fn x() -> Option<MutexGuard<'static, X>> {
    X.get_or_init(|| {
        let (c, screen) = x11rb::connect(None).ok()?;
        let root = c.setup().roots.get(screen)?.root;
        let a = Atoms::new(&c);
        Some(Mutex::new(X { c, root, a, keys: None }))
    })
    .as_ref()?
    .lock()
    .ok()
}

/// Whether Sofa can talk to an X server at all.
pub fn available() -> bool {
    x().is_some()
}

fn prop32(x: &X, w: u32, p: Atom, ty: impl Into<Atom>, len: u32) -> Vec<u32> {
    x.c.get_property(false, w, p, ty, 0, len).ok().and_then(|c| c.reply().ok()).and_then(|r| r.value32().map(|v| v.collect())).unwrap_or_default()
}

fn prop_bytes(x: &X, w: u32, p: Atom, ty: impl Into<Atom>) -> Vec<u8> {
    x.c.get_property(false, w, p, ty, 0, 1024).ok().and_then(|c| c.reply().ok()).map(|r| r.value).unwrap_or_default()
}

fn send(x: &X, w: u32, ty: Atom, data: [u32; 5]) {
    let ev = ClientMessageEvent::new(32, w, ty, data);
    let _ = x.c.send_event(false, x.root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, ev);
    let _ = x.c.flush();
}

/// The window in front (0: none, or a window X11 cannot see).
pub fn active() -> isize {
    let Some(x) = x() else { return 0 };
    if x.a.active != 0
        && let Some(&w) = prop32(&x, x.root, x.a.active, AtomEnum::WINDOW, 1).first()
    {
        return w as isize;
    }
    // No window manager (a QA run under Xvfb): the input focus.
    x.c.get_input_focus().ok().and_then(|c| c.reply().ok()).map_or(0, |r| if r.focus > 1 { r.focus as isize } else { 0 })
}

/// The managed windows, front to back.
pub fn stacking() -> Vec<isize> {
    let Some(x) = x() else { return Vec::new() };
    let mut list = prop32(&x, x.root, x.a.stacking, AtomEnum::WINDOW, 4096);
    if list.is_empty() {
        list = prop32(&x, x.root, x.a.clients, AtomEnum::WINDOW, 4096);
    }
    if list.is_empty() {
        // No window manager: the root's mapped children.
        list = x.c.query_tree(x.root).ok().and_then(|c| c.reply().ok()).map(|t| t.children).unwrap_or_default();
        list.retain(|&w| x.c.get_window_attributes(w).ok().and_then(|c| c.reply().ok()).is_some_and(|a| a.map_state == MapState::VIEWABLE && !a.override_redirect));
    }
    list.reverse();
    list.into_iter().map(|w| w as isize).collect()
}

/// Whether the window is still open.
pub fn exists(w: isize) -> bool {
    if w <= 0 {
        return false;
    }
    if let Ok(g) = CLIENTS.lock()
        && let Some(list) = g.as_ref()
        && !list.is_empty()
    {
        return list.contains(&(w as u32));
    }
    let Some(x) = x() else { return false };
    let list = prop32(&x, x.root, x.a.clients, AtomEnum::WINDOW, 4096);
    if !list.is_empty() {
        return list.contains(&(w as u32));
    }
    x.c.get_window_attributes(w as u32).ok().and_then(|c| c.reply().ok()).is_some_and(|a| a.map_state != MapState::UNMAPPED)
}

pub fn pid(w: isize) -> u32 {
    let Some(x) = x() else { return 0 };
    prop32(&x, w as u32, x.a.pid, AtomEnum::CARDINAL, 1).first().copied().unwrap_or(0)
}

pub fn title(w: isize) -> String {
    let Some(x) = x() else { return String::new() };
    let t = prop_bytes(&x, w as u32, x.a.name, x.a.utf8);
    let t = if t.is_empty() { prop_bytes(&x, w as u32, AtomEnum::WM_NAME.into(), AtomEnum::ANY) } else { t };
    String::from_utf8_lossy(&t).trim_end_matches('\0').to_owned()
}

/// WM_CLASS: the instance and class names ("steamwebhelper", "steam").
pub fn class(w: isize) -> (String, String) {
    let Some(x) = x() else { return Default::default() };
    let v = prop_bytes(&x, w as u32, AtomEnum::WM_CLASS.into(), AtomEnum::STRING);
    let mut parts = v.split(|&b| b == 0).map(|p| String::from_utf8_lossy(p).into_owned());
    (parts.next().unwrap_or_default(), parts.next().unwrap_or_default())
}

fn states(x: &X, w: u32) -> Vec<u32> {
    prop32(x, w, x.a.state, AtomEnum::ATOM, 64)
}

pub fn fullscreen(w: isize) -> bool {
    let Some(x) = x() else { return false };
    if states(&x, w as u32).contains(&x.a.fullscreen) {
        return true;
    }
    // Games that size themselves to the screen without asking for full screen.
    let screen = x.c.get_geometry(x.root).ok().and_then(|c| c.reply().ok());
    let geo = x.c.get_geometry(w as u32).ok().and_then(|c| c.reply().ok());
    let at = x.c.translate_coordinates(w as u32, x.root, 0, 0).ok().and_then(|c| c.reply().ok());
    match (screen, geo, at) {
        (Some(s), Some(g), Some(t)) => t.dst_x <= 0 && t.dst_y <= 0 && i32::from(t.dst_x) + i32::from(g.width) >= i32::from(s.width) && i32::from(t.dst_y) + i32::from(g.height) >= i32::from(s.height),
        _ => false,
    }
}

/// A desktop or a panel (the parts of the desktop Sofa stands in for).
pub fn is_desktop_or_panel(w: isize) -> bool {
    let Some(x) = x() else { return false };
    let t = prop32(&x, w as u32, x.a.kind, AtomEnum::ATOM, 16);
    t.contains(&x.a.desktop) || t.contains(&x.a.dock)
}

/// Brings a window forward (restoring it when minimized), as a pager does.
pub fn activate(w: isize) {
    let Some(x) = x() else { return };
    if x.a.active == 0 {
        let _ = x.c.map_window(w as u32);
        let _ = x.c.set_input_focus(InputFocus::PARENT, w as u32, x11rb::CURRENT_TIME);
        let _ = x.c.flush();
        return;
    }
    send(&x, w as u32, x.a.active, [FROM_TOOL, x11rb::CURRENT_TIME, 0, 0, 0]);
}

pub fn minimize(w: isize) {
    let Some(x) = x() else { return };
    // ICCCM: WM_CHANGE_STATE with IconicState.
    send(&x, w as u32, x.a.change_state, [3, 0, 0, 0, 0]);
}

/// Asks a window to close, as its own close button would.
pub fn close(w: isize) {
    let Some(x) = x() else { return };
    send(&x, w as u32, x.a.close, [x11rb::CURRENT_TIME, FROM_TOOL, 0, 0, 0]);
}

/// Keeps a window above the others, or lets it go back among them.
pub fn set_above(w: isize, on: bool) {
    let Some(x) = x() else { return };
    send(&x, w as u32, x.a.state, [u32::from(on), x.a.above, 0, FROM_TOOL, 0]);
}

/// The screen's size in pixels.
pub fn screen_size() -> (u32, u32) {
    let Some(x) = x() else { return (1920, 1080) };
    x.c.get_geometry(x.root).ok().and_then(|c| c.reply().ok()).map_or((1920, 1080), |g| (u32::from(g.width), u32::from(g.height)))
}

/// The window's own icon (`_NET_WM_ICON`, the largest up to 256 px), as RGBA.
pub fn icon(w: isize) -> Option<(u32, u32, Vec<u8>)> {
    let x = x()?;
    let v = prop32(&x, w as u32, x.a.icon, AtomEnum::CARDINAL, 1 << 20);
    let mut best: Option<(usize, u32, u32)> = None;
    let mut i = 0;
    while i + 2 <= v.len() {
        let (iw, ih) = (v[i], v[i + 1]);
        let n = (iw as usize) * (ih as usize);
        if iw == 0 || ih == 0 || i + 2 + n > v.len() {
            break;
        }
        if iw <= 256 && best.is_none_or(|(_, bw, _)| iw > bw) {
            best = Some((i + 2, iw, ih));
        }
        i += 2 + n;
    }
    let (at, iw, ih) = best?;
    let mut out = Vec::with_capacity((iw * ih * 4) as usize);
    for &argb in &v[at..at + (iw * ih) as usize] {
        out.extend_from_slice(&[(argb >> 16) as u8, (argb >> 8) as u8, argb as u8, (argb >> 24) as u8]);
    }
    Some((iw, ih, out))
}

/// Presses and releases a key in the window in front (the YouTube remote), by keysym.
pub fn send_key(keysym: u32) {
    use x11rb::protocol::xtest::ConnectionExt as _;
    let Some(mut x) = x() else { return };
    if x.keys.is_none() {
        let (min, max) = (x.c.setup().min_keycode, x.c.setup().max_keycode);
        x.keys = x.c.get_keyboard_mapping(min, max - min + 1).ok().and_then(|c| c.reply().ok()).map(|m| (min, m.keysyms_per_keycode, m.keysyms));
    }
    let Some((min, per, syms)) = &x.keys else { return };
    let per = usize::from((*per).max(1));
    let Some(code) = syms.chunks(per).position(|k| k.contains(&keysym)).and_then(|i| u8::try_from(i + usize::from(*min)).ok()) else { return };
    let root = x.root;
    let _ = x.c.xtest_fake_input(KEY_PRESS_EVENT, code, x11rb::CURRENT_TIME, root, 0, 0, 0);
    let _ = x.c.xtest_fake_input(KEY_RELEASE_EVENT, code, x11rb::CURRENT_TIME, root, 0, 0, 0);
    let _ = x.c.flush();
}

/// The clipboard's text: asked of its owner as UTF-8 on a window of Sofa's own, waiting up to
/// half a second (under Wayland, XWayland passes the request on to the Wayland clipboard).
pub fn clipboard_text() -> Option<String> {
    use std::time::{Duration, Instant};
    let (c, screen) = x11rb::connect(None).ok()?;
    let root = c.setup().roots.get(screen)?.root;
    let win = c.generate_id().ok()?;
    c.create_window(x11rb::COPY_DEPTH_FROM_PARENT, win, root, 0, 0, 1, 1, 0, WindowClass::INPUT_OUTPUT, x11rb::COPY_FROM_PARENT, &CreateWindowAux::new()).ok()?;
    let (clipboard, utf8, prop) = (atom(&c, "CLIPBOARD"), atom(&c, "UTF8_STRING"), atom(&c, "SOFA_CLIPBOARD"));
    c.convert_selection(win, clipboard, utf8, prop, x11rb::CURRENT_TIME).ok()?;
    c.flush().ok()?;
    let until = Instant::now() + Duration::from_millis(500);
    while Instant::now() < until {
        match c.poll_for_event() {
            Ok(Some(Event::SelectionNotify(e))) => {
                if e.property == x11rb::NONE {
                    return None;
                }
                let r = c.get_property(true, win, prop, AtomEnum::ANY, 0, 1 << 16).ok()?.reply().ok()?;
                return Some(String::from_utf8_lossy(&r.value).into_owned());
            }
            Ok(Some(_)) => {}
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => return None,
        }
    }
    None
}

/// Watches the window manager on a thread of its own: `changed` gets the window in front each
/// time it changes, and None when only the list of open windows changed (kept for `exists`).
pub fn watch(changed: impl Fn(Option<isize>) + Send + 'static) {
    let _ = std::thread::Builder::new().name("x11watch".into()).spawn(move || {
        let Ok((c, screen)) = x11rb::connect(None) else { return };
        let Some(root) = c.setup().roots.get(screen).map(|s| s.root) else { return };
        let a = Atoms::new(&c);
        if c.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE)).is_err() || c.flush().is_err() {
            return;
        }
        let read = |p: Atom, ty: AtomEnum, len: u32| -> Vec<u32> { c.get_property(false, root, p, ty, 0, len).ok().and_then(|r| r.reply().ok()).and_then(|r| r.value32().map(|v| v.collect())).unwrap_or_default() };
        if let Ok(mut g) = CLIENTS.lock() {
            *g = Some(read(a.clients, AtomEnum::WINDOW, 4096));
        }
        let mut last = read(a.active, AtomEnum::WINDOW, 1).first().copied().unwrap_or(0);
        while let Ok(ev) = c.wait_for_event() {
            let Event::PropertyNotify(p) = ev else { continue };
            if p.atom == a.clients {
                if let Ok(mut g) = CLIENTS.lock() {
                    *g = Some(read(a.clients, AtomEnum::WINDOW, 4096));
                }
                // A window opening or closing: the loop checks the sessions it tracks.
                changed(None);
            } else if p.atom == a.active {
                let w = read(a.active, AtomEnum::WINDOW, 1).first().copied().unwrap_or(0);
                if w != last {
                    last = w;
                    changed(Some(w as isize));
                }
            }
        }
    });
}
