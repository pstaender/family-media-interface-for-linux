//! familynovice-web: a locked-down fullscreen browser window for one website.
//!
//! Started by the media center:
//! `familynovice-web --url <start page> --allow <rule>… [--title <name>] [--data-dir <dir>]`
//! (`--windowed` for development). Rules: `kika.de` (exactly) or `*.kika.de` (with
//! subdomains), see `src/sites.rs`, which is shared with the media center.
//!
//! - fullscreen, no address bar, no menus; only a close button (✕) in the corner,
//! - holding Esc (or a remote's back key) for 3 s closes the window,
//! - pages (main frame) only on the allowed domain: every visited address is checked; a page
//!   outside is stopped and a dialog offers "Go back" or "Close website",
//! - media/scripts/embedded players from anywhere, except local addresses (any frame),
//! - pop-ups open in the same window if allowed, otherwise the same dialog appears,
//! - downloads are blocked (nothing is written; cookies etc. only in `--data-dir`).

// Shared with the media center; parts of it are only used there.
#[path = "../../src/sites.rs"]
#[allow(dead_code)]
mod sites;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, glib};
use sites::Rule;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::unix::{WindowBuilderExtUnix, WindowExtUnix};
use tao::window::{Fullscreen, WindowBuilder};
use webkit2gtk::{LoadEvent, WebViewExt};
use wry::{NewWindowResponse, WebContext, WebViewBuilder, WebViewBuilderExtUnix, WebViewExtUnix};

const ESC_HOLD: Duration = Duration::from_secs(3);

const CSS: &str = "
button.fn-close {
  font-size: 30px; min-width: 64px; min-height: 64px; padding: 0;
  border-radius: 32px; border: 2px solid rgba(255,255,255,0.7);
  background: rgba(0,0,0,0.55); color: #fff; box-shadow: none;
}
button.fn-close:hover { background: rgba(0,0,0,0.85); }
box.fn-dialog {
  background: rgba(22,25,34,0.97); border-radius: 20px; padding: 32px 40px;
  border: 1px solid rgba(255,255,255,0.15);
}
label.fn-dialog-title { font-size: 28px; font-weight: bold; color: #fff; }
label.fn-dialog-text { font-size: 20px; color: #c9cede; }
button.fn-btn {
  font-size: 22px; padding: 12px 28px; border-radius: 12px; box-shadow: none;
  background: #2e3345; color: #fff; border: 2px solid transparent;
}
button.fn-btn:focus { border-color: #fff; }
button.fn-primary { background: #3d8bff; }
label.fn-hint {
  font-size: 24px; color: #fff; background: rgba(0,0,0,0.75);
  padding: 14px 24px; border-radius: 14px;
}
";

struct Args {
    url: String,
    rules: Vec<Rule>,
    title: String,
    data_dir: Option<PathBuf>,
    windowed: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let (mut url, mut rules, mut title, mut data_dir, mut windowed) =
        (None, Vec::new(), String::from("familynovice"), None, false);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--url" => url = Some(value()?),
            "--allow" => {
                let v = value()?;
                rules.push(Rule::parse(&v).ok_or(format!("invalid --allow rule: {v}"))?);
            }
            "--title" => title = value()?,
            "--data-dir" => data_dir = Some(PathBuf::from(value()?)),
            "--windowed" => windowed = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    let url = url.ok_or("--url is required")?;
    if rules.is_empty() {
        return Err("at least one --allow rule is required".into());
    }
    if !sites::is_allowed_page(&url, &rules) {
        return Err(format!("the start page {url} is not allowed by the rules"));
    }
    Ok(Args {
        url,
        rules,
        title,
        data_dir,
        windowed,
    })
}

enum UserEvent {
    Close,
}

/// The dialog shown when a page outside the website is opened.
#[derive(Clone)]
struct BlockedDialog {
    container: gtk::Box,
    address: gtk::Label,
    back_button: gtk::Button,
    close_button: gtk::Button,
}

impl BlockedDialog {
    fn new(site_name: &str) -> Self {
        let container = gtk::Box::new(gtk::Orientation::Vertical, 14);
        container.style_context().add_class("fn-dialog");
        container.set_halign(gtk::Align::Center);
        container.set_valign(gtk::Align::Center);
        let title = gtk::Label::new(Some(&format!("This page is not part of {site_name}")));
        title.style_context().add_class("fn-dialog-title");
        title.set_line_wrap(true);
        title.set_max_width_chars(40);
        let address = gtk::Label::new(None);
        address.style_context().add_class("fn-dialog-text");
        address.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        address.set_max_width_chars(50);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        buttons.set_halign(gtk::Align::Center);
        buttons.set_margin_top(12);
        let back_button = gtk::Button::with_label("Go back");
        back_button.style_context().add_class("fn-btn");
        back_button.style_context().add_class("fn-primary");
        let close_button = gtk::Button::with_label("Close website");
        close_button.style_context().add_class("fn-btn");
        buttons.pack_start(&back_button, false, false, 0);
        buttons.pack_start(&close_button, false, false, 0);
        container.pack_start(&title, false, false, 0);
        container.pack_start(&address, false, false, 0);
        container.pack_start(&buttons, false, false, 0);
        container.set_no_show_all(true);
        Self {
            container,
            address,
            back_button,
            close_button,
        }
    }

    /// Show the dialog for a blocked address (only its host is shown, e.g. "evil.com").
    fn show(&self, url: &str) {
        let host = url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(String::from))
            .unwrap_or_else(|| url.to_string());
        self.address.set_text(&host);
        // show_all() does nothing on a widget with no_show_all set (which keeps the dialog
        // hidden when the window first appears), so lift it here.
        self.container.set_no_show_all(false);
        self.container.show_all();
        self.back_button.grab_focus(); // Enter = go back
    }

    fn hide(&self) {
        self.container.hide();
    }
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!(
                "familynovice-web: {e}\n\
                 usage: familynovice-web --url URL --allow RULE [--allow RULE…] \
                 [--title NAME] [--data-dir DIR] [--windowed]"
            );
            std::process::exit(2);
        }
    };
    if let Err(e) = run(args) {
        eprintln!("familynovice-web: {e}");
        std::process::exit(1);
    }
}

fn run(args: Args) -> Result<(), String> {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = WindowBuilder::new()
        .with_title(&args.title)
        .with_default_vbox(false)
        .with_decorations(args.windowed)
        .with_inner_size(tao::dpi::LogicalSize::new(1280.0, 720.0))
        .with_fullscreen((!args.windowed).then_some(Fullscreen::Borderless(None)))
        .build(&event_loop)
        .map_err(|e| format!("cannot open window: {e}"))?;

    // Layout: the page fills the window; close button and Esc hint float above it.
    let provider = gtk::CssProvider::new();
    provider
        .load_from_data(CSS.as_bytes())
        .map_err(|e| format!("css: {e}"))?;
    if let Some(screen) = gdk::Screen::default() {
        gtk::StyleContext::add_provider_for_screen(
            &screen,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    let overlay = gtk::Overlay::new();
    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    overlay.add(&page);
    window.gtk_window().add(&overlay);

    let close = gtk::Button::with_label("✕");
    close.style_context().add_class("fn-close");
    close.set_halign(gtk::Align::End);
    close.set_valign(gtk::Align::Start);
    close.set_margin_top(20);
    close.set_margin_end(20);
    close.set_tooltip_text(Some("Close"));
    close.set_can_focus(false); // keyboard focus stays in the page
    overlay.add_overlay(&close);
    let p = proxy.clone();
    close.connect_clicked(move |_| {
        let _ = p.send_event(UserEvent::Close);
    });

    let hint = gtk::Label::new(Some("Keep holding Esc to close…"));
    hint.style_context().add_class("fn-hint");
    hint.set_halign(gtk::Align::Center);
    hint.set_valign(gtk::Align::Start);
    hint.set_margin_top(28);
    hint.set_no_show_all(true);
    overlay.add_overlay(&hint);
    overlay.set_overlay_pass_through(&hint, true);

    // "Not allowed" dialog: shown whenever a page outside the website is opened.
    let dialog = BlockedDialog::new(&args.title);
    overlay.add_overlay(&dialog.container);
    {
        let p = proxy.clone();
        dialog.close_button.connect_clicked(move |_| {
            let _ = p.send_event(UserEvent::Close);
        });
    }

    // The page.
    let rules = Rc::new(args.rules);
    let view_slot: Rc<RefCell<Option<webkit2gtk::WebView>>> = Rc::default();
    let mut context = WebContext::new(args.data_dir);
    let webview = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(&args.url)
        .with_devtools(false)
        // Any frame: block local addresses and non-web schemes.
        .with_navigation_handler(|url| {
            let blocked = sites::is_always_blocked(&url);
            if blocked {
                eprintln!("familynovice-web: blocked {url}");
            }
            !blocked
        })
        // Pop-ups: open allowed pages in this window, never a new one.
        .with_new_window_req_handler({
            let (rules, slot) = (rules.clone(), view_slot.clone());
            let dialog_for_popups = dialog.clone();
            move |url, _features| {
                if sites::is_allowed_page(&url, &rules) {
                    if let Some(view) = slot.borrow().as_ref() {
                        view.load_uri(&url);
                    }
                } else {
                    eprintln!("familynovice-web: blocked pop-up {url}");
                    dialog_for_popups.show(&url);
                }
                NewWindowResponse::Deny
            }
        })
        // Non-destructive: never write downloads.
        .with_download_started_handler(|url, _path| {
            eprintln!("familynovice-web: blocked download {url}");
            false
        })
        .build_gtk(&page)
        .map_err(|e| format!("cannot create web view: {e}"))?;
    let view = webview.webview();
    *view_slot.borrow_mut() = Some(view.clone());

    // Main frame: only pages on the allowed domain. `uri` is the main frame's (provisional)
    // address; stopping before the commit keeps the current page on screen.
    {
        let (rules, dialog) = (rules.clone(), dialog.clone());
        view.connect_load_changed(move |view, event| {
            if !matches!(
                event,
                LoadEvent::Started | LoadEvent::Redirected | LoadEvent::Committed
            ) {
                return;
            }
            let uri = view.uri().map(|u| u.to_string()).unwrap_or_default();
            if sites::is_allowed_page(&uri, &rules) {
                return;
            }
            eprintln!("familynovice-web: blocked page {uri}");
            view.stop_loading();
            dialog.show(&uri);
        });
    }
    // "Go back": back to the last allowed page (the blocked load never committed, so normally
    // the current page is still the allowed one).
    {
        let (rules, dialog_c, start, view) = (
            rules.clone(),
            dialog.clone(),
            args.url.clone(),
            view.clone(),
        );
        dialog.back_button.connect_clicked(move |_| {
            dialog_c.hide();
            let current = view.uri().map(|u| u.to_string()).unwrap_or_default();
            if !sites::is_allowed_page(&current, &rules) {
                if view.can_go_back() {
                    view.go_back();
                } else {
                    view.load_uri(&start);
                }
            }
            view.grab_focus();
        });
    }

    // Holding Esc (or the remote's back key) for 3 s closes the window. Short presses still
    // reach the page (e.g. to leave a video's fullscreen mode).
    let esc_since: Rc<Cell<Option<Instant>>> = Rc::default();
    let is_close_key = |key: gdk::keys::Key| {
        key == gdk::keys::constants::Escape || key == gdk::keys::constants::Back
    };
    {
        let (esc_since, hint) = (esc_since.clone(), hint.clone());
        window.gtk_window().connect_key_press_event(move |_, ev| {
            if is_close_key(ev.keyval()) && esc_since.get().is_none() {
                esc_since.set(Some(Instant::now()));
                hint.show();
            }
            glib::Propagation::Proceed
        });
    }
    {
        let (esc_since, hint) = (esc_since.clone(), hint.clone());
        window.gtk_window().connect_key_release_event(move |_, ev| {
            if is_close_key(ev.keyval()) {
                esc_since.set(None);
                hint.hide();
            }
            glib::Propagation::Proceed
        });
    }
    {
        let p = proxy.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            if esc_since.get().is_some_and(|t| t.elapsed() >= ESC_HOLD) {
                let _ = p.send_event(UserEvent::Close);
            }
            glib::ControlFlow::Continue
        });
    }

    window.gtk_window().show_all();
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        let _keep_alive = (&webview, &context, &window);
        match event {
            Event::UserEvent(UserEvent::Close)
            | Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,
            _ => {}
        }
    })
}
