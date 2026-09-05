use adw::prelude::*;
use cleanly_core::*;
use cleanly_platform::files;
use cleanly_service::{Discovery, OperationResult, Service};
use gtk::{gio, glib};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};
const APP_ID: &str = "io.github.cleanly.Cleanly";
struct Ui {
    window: adw::ApplicationWindow,
    split: adw::OverlaySplitView,
    list: gtk::ListBox,
    search: gtk::SearchEntry,
    filter: gtk::DropDown,
    sort: gtk::DropDown,
    detail: gtk::Box,
    footer: gtk::Box,
    status: gtk::Label,
    apps: RefCell<Vec<InstalledApp>>,
    manifest: RefCell<Option<AppManifest>>,
    service: Service,
    generation: Cell<u64>,
    scan_generation: Cell<u64>,
    inspection: RefCell<Cancellation>,
    discovery: RefCell<Cancellation>,
    busy: Cell<bool>,
    backend_errors: RefCell<Vec<String>>,
}
fn main() {
    if std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("Uid:")).map(String::from))
        .is_some_and(|s| s.split_whitespace().nth(2) == Some("0"))
    {
        eprintln!(
            "Cleanly must run as your normal user. Only the installed helper may run as root."
        );
        std::process::exit(1);
    }
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(if std::env::var_os("CLEANLY_SMOKE_TEST").is_some() {
            gio::ApplicationFlags::NON_UNIQUE
        } else {
            gio::ApplicationFlags::empty()
        })
        .build();
    app.connect_activate(build);
    app.run();
}
fn label(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.);
    l.set_wrap(true);
    if !class.is_empty() {
        l.add_css_class(class);
    }
    l
}
fn margins(w: &impl IsA<gtk::Widget>, n: i32) {
    w.set_margin_top(n);
    w.set_margin_bottom(n);
    w.set_margin_start(n);
    w.set_margin_end(n);
}
fn action_row(title: &str, subtitle: &str) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_use_markup(false);
    row.set_title(title);
    row.set_subtitle(subtitle);
    row.set_subtitle_selectable(true);
    row
}
fn build(app: &adw::Application) {
    let css = gtk::CssProvider::new();
    css.load_from_string(include_str!("style.css"));
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Cleanly")
        .default_width(1100)
        .default_height(760)
        .width_request(390)
        .height_request(480)
        .build();
    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let sh = adw::HeaderBar::new();
    sh.set_title_widget(Some(&adw::WindowTitle::new("Cleanly", "APPLICATIONS")));
    sh.set_show_end_title_buttons(false);
    let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh.set_tooltip_text(Some("Refresh applications (Ctrl+R)"));
    sh.pack_start(&refresh);
    let menu = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Preferences and history")
        .build();
    sh.pack_end(&menu);
    sidebar.append(&sh);
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search applications")
        .hexpand(true)
        .build();
    search.set_search_delay(0);
    margins(&search, 12);
    sidebar.append(&search);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    controls.set_margin_start(12);
    controls.set_margin_end(12);
    controls.set_margin_bottom(12);
    let filter = gtk::DropDown::from_strings(&[
        "All applications",
        "APT",
        "Flatpak",
        "Snap",
        "AppImage",
        "Standalone",
    ]);
    filter.set_hexpand(true);
    filter.set_tooltip_text(Some("Filter installation type"));
    let sort = gtk::DropDown::from_strings(&["Name", "Size", "Source"]);
    sort.set_tooltip_text(Some(
        "Sort applications. Installation dates are not reliably available.",
    ));
    controls.append(&filter);
    controls.append(&sort);
    sidebar.append(&controls);
    let list = gtk::ListBox::new();
    list.add_css_class("navigation-sidebar");
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_activate_on_single_click(true);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&list)
        .build();
    sidebar.append(&scroll);
    let status = label("Discovering installed applications…", "dim-label");
    margins(&status, 16);
    sidebar.append(&status);
    let content = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(
        "Application Inspector",
        "Inspect first. Remove with confidence.",
    )));
    let back = gtk::Button::from_icon_name("sidebar-show-symbolic");
    back.set_tooltip_text(Some("Show applications"));
    header.pack_start(&back);
    content.add_top_bar(&header);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    margins(&footer, 16);
    content.add_bottom_bar(&footer);
    let detail = gtk::Box::new(gtk::Orientation::Vertical, 24);
    margins(&detail, 28);
    let clamp = adw::Clamp::builder()
        .maximum_size(780)
        .tightening_threshold(600)
        .child(&detail)
        .build();
    content.set_content(Some(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&clamp)
            .build(),
    ));
    let split = adw::OverlaySplitView::builder()
        .sidebar(&sidebar)
        .content(&content)
        .min_sidebar_width(290.)
        .max_sidebar_width(360.)
        .sidebar_width_fraction(0.31)
        .build();
    window.set_content(Some(&split));
    let breakpoint =
        adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 760sp").unwrap());
    breakpoint.add_setter(&split, "collapsed", Some(&true.to_value()));
    window.add_breakpoint(breakpoint);
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let ui = Rc::new(Ui {
        window,
        split,
        list,
        search,
        filter,
        sort,
        detail,
        footer,
        status,
        apps: RefCell::new(vec![]),
        manifest: RefCell::new(None),
        service: Service::new(home),
        generation: Cell::new(0),
        scan_generation: Cell::new(0),
        inspection: RefCell::new(Cancellation::default()),
        discovery: RefCell::new(Cancellation::default()),
        busy: Cell::new(false),
        backend_errors: RefCell::new(vec![]),
    });
    ui.empty();
    let weak = Rc::downgrade(&ui);
    ui.list.set_filter_func(move |row| {
        let Some(ui) = weak.upgrade() else {
            return false;
        };
        let apps = ui.apps.borrow();
        let Some(app) = row
            .widget_name()
            .parse::<usize>()
            .ok()
            .and_then(|i| apps.get(i))
        else {
            return false;
        };
        let kind = ui.filter.selected();
        let source = match kind {
            1 => Some(Backend::Apt),
            2 => Some(Backend::Flatpak),
            3 => Some(Backend::Snap),
            4 => Some(Backend::AppImage),
            5 => Some(Backend::Manual),
            _ => None,
        };
        let search = ui.search.text().to_lowercase();
        source.is_none_or(|k| app.backend == k)
            && (app.name.to_lowercase().contains(&search)
                || app.id.to_lowercase().contains(&search))
    });
    let weak = Rc::downgrade(&ui);
    ui.list.set_sort_func(move |a, b| {
        let Some(ui) = weak.upgrade() else {
            return gtk::Ordering::Equal;
        };
        let apps = ui.apps.borrow();
        let a = a
            .widget_name()
            .parse::<usize>()
            .ok()
            .and_then(|i| apps.get(i));
        let b = b
            .widget_name()
            .parse::<usize>()
            .ok()
            .and_then(|i| apps.get(i));
        let order = match (a, b) {
            (Some(a), Some(b)) => match ui.sort.selected() {
                1 => b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)),
                2 => a
                    .backend
                    .label()
                    .cmp(b.backend.label())
                    .then_with(|| a.name.cmp(&b.name)),
                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            },
            _ => std::cmp::Ordering::Equal,
        };
        order.into()
    });
    let weak = Rc::downgrade(&ui);
    ui.list.connect_row_selected(move |_, row| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        if ui.busy.get() {
            return;
        }
        if let Some(row) = row {
            let app = row
                .widget_name()
                .parse::<usize>()
                .ok()
                .and_then(|i| ui.apps.borrow().get(i).cloned());
            if let Some(app) = app {
                ui.select(app);
            }
        }
    });
    let weak = Rc::downgrade(&ui);
    ui.search.connect_search_changed(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.list.invalidate_filter();
        }
    });
    let weak = Rc::downgrade(&ui);
    ui.filter.connect_selected_notify(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.list.invalidate_filter();
        }
    });
    let weak = Rc::downgrade(&ui);
    ui.sort.connect_selected_notify(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.list.invalidate_sort();
        }
    });
    let weak = Rc::downgrade(&ui);
    refresh.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.scan();
        }
    });
    let weak = Rc::downgrade(&ui);
    back.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.split.set_show_sidebar(!ui.split.shows_sidebar());
        }
    });
    let model = gio::Menu::new();
    model.append(Some("History & Quarantine"), Some("win.history"));
    model.append(Some("Preferences"), Some("win.preferences"));
    model.append(Some("About Cleanly"), Some("win.about"));
    menu.set_menu_model(Some(&model));
    for name in ["history", "preferences", "about", "refresh", "search"] {
        let action = gio::SimpleAction::new(name, None);
        let weak = Rc::downgrade(&ui);
        action.connect_activate(move |action, _| {
            if let Some(ui) = weak.upgrade() {
                match action.name().as_str() {
                    "history" => ui.history(),
                    "preferences" => ui.preferences(),
                    "about" => ui.about(),
                    "refresh" => ui.scan(),
                    "search" => {
                        ui.split.set_show_sidebar(true);
                        ui.search.grab_focus();
                    }
                    _ => {}
                }
            }
        });
        ui.window.add_action(&action);
    }
    app.set_accels_for_action("win.search", &["<Control>f"]);
    app.set_accels_for_action("win.refresh", &["<Control>r"]);
    let keep = ui.clone();
    ui.window.connect_close_request(move |_| {
        if keep.busy.get() {
            keep.message(
                "Operation in progress",
                "Keep Cleanly open until package verification and history writing finish.",
            );
            glib::Propagation::Stop
        } else {
            keep.inspection.borrow().cancel();
            keep.discovery.borrow().cancel();
            glib::Propagation::Proceed
        }
    });
    adw::StyleManager::default().set_color_scheme(match ui.service.appearance() {
        1 => adw::ColorScheme::ForceLight,
        2 => adw::ColorScheme::ForceDark,
        _ => adw::ColorScheme::Default,
    });
    if std::env::var("CLEANLY_SMOKE_VIEW").as_deref() == Ok("narrow") {
        ui.window.set_default_size(430, 760);
    }
    if std::env::var("CLEANLY_SMOKE_VIEW").as_deref() == Ok("light") {
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceLight);
    }
    ui.window.present();
    ui.scan();
    if std::env::var_os("CLEANLY_SMOKE_TEST").is_some() {
        smoke_test(ui.clone());
    }
}
impl Ui {
    fn clear(&self) {
        while let Some(child) = self.footer.first_child() {
            self.footer.remove(&child);
        }
        while let Some(child) = self.detail.first_child() {
            self.detail.remove(&child);
        }
    }
    fn empty(&self) {
        self.clear();
        let page=adw::StatusPage::builder().icon_name("edit-clear-all-symbolic").title("A little clarity.\nA cleaner system.").description("Choose an application to inspect its files, understand what belongs to it, and review a safe removal plan.").vexpand(true).build();
        self.detail.append(&page);
        let note = label(
            "Your files come first. Uncertain ownership always means keep.",
            "dim-label",
        );
        note.set_halign(gtk::Align::Center);
        self.detail.append(&note);
    }
    fn message(&self, title: &str, body: &str) {
        let dialog = adw::AlertDialog::builder()
            .heading(title)
            .body(body)
            .build();
        dialog.add_response("ok", "OK");
        dialog.set_default_response(Some("ok"));
        dialog.present(Some(&self.window));
    }
    fn scan(self: &Rc<Self>) {
        if self.busy.get() {
            return;
        }
        self.discovery.borrow().cancel();
        let cancel = Cancellation::default();
        *self.discovery.borrow_mut() = cancel.clone();
        self.inspection.borrow().cancel();
        self.generation.set(self.generation.get() + 1);
        *self.manifest.borrow_mut() = None;
        self.apps.borrow_mut().clear();
        self.backend_errors.borrow_mut().clear();
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        self.status.set_label("Discovering installed applications…");
        self.empty();
        self.scan_generation.set(self.scan_generation.get() + 1);
        if let Ok(apps) = self.service.cached_apps() {
            for backend in [
                Backend::Apt,
                Backend::Flatpak,
                Backend::Snap,
                Backend::AppImage,
                Backend::Manual,
            ] {
                self.add_discovery(
                    Discovery {
                        backend,
                        apps: apps
                            .iter()
                            .filter(|a| a.backend == backend)
                            .cloned()
                            .collect(),
                        error: None,
                    },
                    0,
                );
            }
            self.status
                .set_label("Cached applications · refreshing ownership metadata…");
        }
        let service = self.service.clone();
        let (tx, rx) = async_channel::unbounded();
        std::thread::spawn(move || {
            service.discover(
                |d| {
                    let _ = tx.send_blocking(d);
                },
                &cancel,
            );
        });
        let weak = Rc::downgrade(self);
        let generation = self.scan_generation.get();
        glib::spawn_future_local(async move {
            let mut completed = 0;
            while let Ok(discovery) = rx.recv().await {
                let Some(ui) = weak.upgrade() else {
                    return;
                };
                if generation != ui.scan_generation.get() {
                    return;
                }
                completed += 1; // Keep row indices stable: replace cached entries for this backend by rebuilding the list.
                let retained: Vec<_> = ui
                    .apps
                    .borrow()
                    .iter()
                    .filter(|a| a.backend != discovery.backend)
                    .cloned()
                    .collect();
                ui.apps.borrow_mut().clear();
                while let Some(child) = ui.list.first_child() {
                    ui.list.remove(&child);
                }
                ui.add_discovery(
                    Discovery {
                        backend: Backend::Manual,
                        apps: retained,
                        error: None,
                    },
                    completed,
                );
                ui.add_discovery(discovery, completed);
                if completed == 5 {
                    let apps = ui.apps.borrow().clone();
                    let service = ui.service.clone();
                    std::thread::spawn(move || {
                        let _ = service.cache_apps(apps);
                    });
                }
            }
        });
    }
    fn add_discovery(&self, d: Discovery, completed: usize) {
        if let Some(error) = d.error {
            self.backend_errors
                .borrow_mut()
                .push(format!("{}: {error}", d.backend.label()));
        }
        let base = self.apps.borrow().len();
        self.apps.borrow_mut().extend(d.apps.clone());
        for (offset, app) in d.apps.iter().enumerate() {
            let row = gtk::ListBoxRow::new();
            row.set_widget_name(&(base + offset).to_string());
            let body = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            margins(&body, 9);
            let icon = gtk::Image::from_icon_name(if app.icon.starts_with('/') {
                "application-x-executable-symbolic"
            } else {
                &app.icon
            });
            icon.set_pixel_size(36);
            body.append(&icon);
            let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
            text.set_hexpand(true);
            let name = label(&app.name, "heading");
            name.set_wrap(false);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            text.append(&name);
            let sub = label(
                &format!("{}  ·  {}", app.backend.label(), app.version),
                "caption",
            );
            sub.add_css_class("dim-label");
            sub.set_wrap(false);
            sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
            text.append(&sub);
            body.append(&text);
            let size = label(&format_size(app.size), "caption");
            size.add_css_class("dim-label");
            size.set_wrap(false);
            body.append(&size);
            row.set_child(Some(&body));
            self.list.append(&row);
        }
        let errors = self.backend_errors.borrow();
        self.status.set_label(&format!(
            "{} applications{}{}",
            self.apps.borrow().len(),
            if completed < 5 {
                " · Still discovering…"
            } else {
                ""
            },
            if errors.is_empty() {
                String::new()
            } else {
                format!("\n{} source(s) unavailable", errors.len())
            }
        ));
        self.status.set_tooltip_text(Some(&errors.join("\n\n")));
    }
    fn select(self: &Rc<Self>, app: InstalledApp) {
        self.inspection.borrow().cancel();
        let cancel = Cancellation::default();
        *self.inspection.borrow_mut() = cancel.clone();
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        *self.manifest.borrow_mut() = None;
        self.clear();
        let spinner = gtk::Spinner::new();
        spinner.start();
        spinner.set_size_request(40, 40);
        spinner.set_margin_top(100);
        self.detail.append(&spinner);
        self.detail
            .append(&label(&format!("Inspecting {}", app.name), "title-2"));
        self.detail.append(&label(
            "Reading authoritative metadata and checking ownership…",
            "dim-label",
        ));
        if self.split.is_collapsed() {
            self.split.set_show_sidebar(false);
        }
        let service = self.service.clone();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                service.inspect(&app, &cancel)
            }))
            .unwrap_or_else(|_| Err("Inspection worker failed unexpectedly".into()));
            let _ = tx.send_blocking(result);
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Ok(result) = rx.recv().await else {
                return;
            };
            let Some(ui) = weak.upgrade() else {
                return;
            };
            if ui.generation.get() != generation {
                return;
            }
            match result {
                Ok(manifest) => {
                    *ui.manifest.borrow_mut() = Some(manifest.clone());
                    ui.show_manifest(&manifest);
                    if std::env::var("CLEANLY_SMOKE_VIEW").as_deref() == Ok("inspector") {
                        ui.inspector(manifest);
                    }
                    if std::env::var("CLEANLY_SMOKE_VIEW").as_deref() == Ok("confirm") {
                        ui.confirm(RemovalMode::Uninstall);
                    }
                }
                Err(e) => {
                    ui.clear();
                    ui.detail.append(
                        &adw::StatusPage::builder()
                            .icon_name("dialog-warning-symbolic")
                            .title("Inspection unavailable")
                            .description(&e)
                            .build(),
                    );
                }
            }
        });
    }
    fn show_manifest(self: &Rc<Self>, manifest: &AppManifest) {
        self.clear();
        let app = &manifest.app;
        let hero = gtk::Box::new(gtk::Orientation::Horizontal, 20);
        let icon = gtk::Image::from_icon_name(if app.icon.starts_with('/') {
            "application-x-executable-symbolic"
        } else {
            &app.icon
        });
        icon.set_pixel_size(80);
        hero.append(&icon);
        let titles = gtk::Box::new(gtk::Orientation::Vertical, 6);
        titles.set_hexpand(true);
        titles.append(&label(&app.name, "title-1"));
        titles.append(&label(
            &format!("{}  ·  {} installation", app.backend.label(), app.scope),
            "dim-label",
        ));
        let badge = label(
            if app.protection.is_some() {
                "🔒  SYSTEM / PROTECTED"
            } else {
                "✓  Ownership inspected"
            },
            "caption",
        );
        badge.add_css_class(if app.protection.is_some() {
            "warning"
        } else {
            "success"
        });
        titles.append(&badge);
        hero.append(&titles);
        self.detail.append(&hero);
        let overview = adw::PreferencesGroup::builder().title("Overview").build();
        overview.add(&action_row("Package identifier", &app.id));
        overview.add(&action_row(
            "Version & architecture",
            &format!("{} · {}", app.version, app.architecture),
        ));
        overview.add(&action_row("Publisher / source", &app.publisher));
        overview.add(&action_row(
            "Installation location",
            if app.location.as_os_str().is_empty() {
                "Unknown"
            } else {
                app.location.to_str().unwrap_or("Non-UTF8 path")
            },
        ));
        self.detail.append(&overview);
        let storage = adw::PreferencesGroup::builder()
            .title("Storage")
            .description("Metadata estimates where available. Unknown is never counted as zero.")
            .build();
        storage.add(&action_row("Application", &format_size(app.size)));
        for (category, title) in [
            (FileCategory::Configuration, "Configuration"),
            (FileCategory::Cache, "Cache"),
            (FileCategory::Data, "Application data"),
            (FileCategory::State, "State"),
        ] {
            let candidates: Vec<_> = manifest
                .files
                .iter()
                .filter(|f| f.category == category)
                .collect();
            if !candidates.is_empty() {
                let bytes = candidates
                    .iter()
                    .try_fold(0u64, |a, f| f.size.and_then(|b| a.checked_add(b)));
                storage.add(&action_row(title, &format_size(bytes)));
            }
        }
        storage.add(&action_row(
            "Documents & unproven leftovers",
            "Protected · kept · not scanned",
        ));
        self.detail.append(&storage);
        let inspect = gtk::Button::builder()
            .label(format!("Review Files  ·  {} entries", manifest.files.len()))
            .halign(gtk::Align::Start)
            .build();
        let weak = Rc::downgrade(self);
        inspect.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade()
                && let Some(manifest) = ui.manifest.borrow().clone()
            {
                ui.inspector(manifest);
            }
        });
        self.detail.append(&inspect);
        let notes = adw::ExpanderRow::builder()
            .use_markup(false)
            .title("Safety and backend details")
            .subtitle("See what Cleanly knows, and what it preserves")
            .build();
        for note in &manifest.notes {
            notes.add_row(&action_row("", note));
        }
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.append(&notes);
        self.detail.append(&list);
        if let Some(reason) = &app.protection {
            let note = label(reason, "warning");
            self.detail.append(&note);
        }
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        actions.set_halign(gtk::Align::End);
        let complete = gtk::Button::with_label("Complete Removal…");
        let uninstall = gtk::Button::with_label("Uninstall…");
        uninstall.add_css_class("destructive-action");
        for button in [&complete, &uninstall] {
            button.set_sensitive(app.protection.is_none());
        }
        if !manifest
            .files
            .iter()
            .any(|f| f.cleanup_allowed() && f.category != FileCategory::Application)
        {
            complete.set_sensitive(false);
            complete.set_tooltip_text(Some("No additional verified data is eligible for cleanup"));
        }
        actions.append(&complete);
        actions.append(&uninstall);
        actions.set_hexpand(true);
        self.footer.append(&actions);
        for (button, mode) in [
            (complete, RemovalMode::Complete),
            (uninstall, RemovalMode::Uninstall),
        ] {
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.confirm(mode);
                }
            });
        }
    }
    fn inspector(&self, manifest: AppManifest) {
        let dialog = adw::Dialog::builder()
            .title(format!("Files associated with {}", manifest.app.name))
            .content_width(820)
            .content_height(650)
            .build();
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        margins(&body, 16);
        body.append(&label("Every association has a reason", "title-2"));
        body.append(&label("Package files are managed by their backend. Protected and review-only entries cannot be selected for cleanup.","dim-label"));
        let model = gtk::StringList::new(&[]);
        for i in 0..manifest.files.len() {
            model.append(&i.to_string());
        }
        let files = Rc::new(manifest.files);
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let expander = gtk::Expander::new(None);
            margins(&expander, 10);
            let title = label("", "heading");
            title.set_wrap(false);
            title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            expander.set_label_widget(Some(&title));
            let child = label("", "");
            child.set_selectable(true);
            child.set_margin_top(8);
            child.set_margin_bottom(10);
            expander.set_child(Some(&child));
            item.set_child(Some(&expander));
        });
        factory.connect_bind(move|_,item|{let Some(item)=item.downcast_ref::<gtk::ListItem>()else{return;};let Some(index)=item.item().and_downcast::<gtk::StringObject>().and_then(|s|s.string().parse::<usize>().ok())else{return;};let file=&files[index];let Some(expander)=item.child().and_downcast::<gtk::Expander>()else{return;};expander.set_expanded(false);
 let badge=if file.category==FileCategory::Protected{"🔒 Protected"}else{match file.confidence{Confidence::Verified=>"✓ Verified",Confidence::Strong=>"✓ Strong match",Confidence::Weak=>"! Review only",Confidence::Unknown=>"🔒 Unknown — kept"}};
 if let Some(title)=expander.label_widget().and_downcast::<gtk::Label>(){title.set_label(&format!("{}  ·  {badge}",file.path.display()));}
 if let Some(child)=expander.child().and_downcast::<gtk::Label>(){child.set_label(&format!("{}\n\nCategory: {:?}\nSize: {}\nOwnership: {badge}\n\n{}\n\n{}",file.path.display(),file.category,format_size(file.size),file.ownership.description(),if file.cleanup_allowed(){"Eligible for quarantine only after confirmation."}else{"No manual cleanup. Package-owned files may be removed by the package manager as previewed."}));}});
        let selection = gtk::NoSelection::new(Some(model));
        let list = gtk::ListView::new(Some(selection), Some(factory));
        list.add_css_class("boxed-list");
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&list)
            .build();
        body.append(&scroll);
        toolbar.set_content(Some(&body));
        dialog.set_child(Some(&toolbar));
        dialog.present(Some(&self.window));
    }
    fn confirm(self: &Rc<Self>, mode: RemovalMode) {
        let Some(manifest) = self.manifest.borrow().clone() else {
            return;
        };
        let dialog=adw::AlertDialog::builder().heading(format!("Remove {}?",manifest.app.name)).body(format!("{} · {}\n\nThe exact application is requested. No automatic dependency or runtime cleanup. Documents and uncertain leftovers stay protected.",manifest.app.backend.label(),manifest.app.id)).build();
        dialog.add_responses(&[("cancel", "Cancel"), ("remove", "Review Plan")]);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let choices = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let mut checks = Vec::new();
        for (i, file) in manifest
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| f.cleanup_allowed())
        {
            let check = gtk::CheckButton::with_label(&format!(
                "Quarantine {} · {}",
                file.path.display(),
                format_size(file.size)
            ));
            let required = manifest.app.backend == Backend::AppImage
                && file.category == FileCategory::Application;
            check.set_active(required || mode == RemovalMode::Complete);
            check.set_sensitive(!required);
            choices.append(&check);
            checks.push((i, check));
        }
        choices.append(&label("Quarantine keeps data recoverable until you restore or manage it manually. It does not free disk space.","dim-label"));
        dialog.set_extra_child(Some(&choices));
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response != "remove" {
                return;
            }
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let selected = checks
                .iter()
                .filter(|(_, c)| c.is_active())
                .map(|(i, _)| *i)
                .collect();
            ui.prepare_confirmation(manifest.clone(), mode, selected);
        });
        dialog.present(Some(&self.window));
    }
    fn prepare_confirmation(
        self: &Rc<Self>,
        manifest: AppManifest,
        mode: RemovalMode,
        selected: Vec<usize>,
    ) {
        if self.busy.replace(true) {
            return;
        }
        let service = self.service.clone();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(service.prepare(
                manifest,
                mode,
                selected,
                &Cancellation::default(),
            ));
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = rx.recv().await;
            let Some(ui) = weak.upgrade() else {
                return;
            };
            ui.busy.set(false);
            match result {
                Ok(Ok(prepared)) => {
                    let plan = prepared.plan();
                    let app = &plan.manifest().app;
                    let paths = plan
                        .selected()
                        .iter()
                        .map(|&i| plan.manifest().files[i].path.display().to_string())
                        .collect::<Vec<_>>()
                        .join("\n");
                    let dialog=adw::AlertDialog::builder().heading("Confirm removal plan").body(format!("Application: {}\nBackend: {} ({})\nExact identifier: {}\n\nQuarantine:\n{}\n\nThis plan will be revalidated immediately before execution. Package removal itself cannot be undone by restoring data.",app.name,app.backend.label(),app.scope,app.id,if paths.is_empty(){"None — personal data preserved"}else{&paths})).build();
                    dialog.add_responses(&[("cancel", "Cancel"), ("remove", "Uninstall")]);
                    dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
                    dialog.set_default_response(Some("cancel"));
                    dialog.set_close_response("cancel");
                    let slot = Rc::new(RefCell::new(Some(prepared)));
                    let weak = Rc::downgrade(&ui);
                    dialog.connect_response(None, move |_, response| {
                        if response == "remove"
                            && let (Some(ui), Some(plan)) =
                                (weak.upgrade(), slot.borrow_mut().take())
                        {
                            ui.execute(plan);
                        }
                    });
                    dialog.present(Some(&ui.window));
                }
                Ok(Err(e)) => ui.message("Plan could not be prepared", &e),
                Err(_) => ui.message("Worker stopped", "No operation was started."),
            }
        });
    }
    fn execute(self: &Rc<Self>, plan: cleanly_service::PreparedPlan) {
        if self.busy.replace(true) {
            return;
        }
        self.inspection.borrow().cancel();
        self.generation.set(self.generation.get() + 1);
        self.clear();
        self.list.set_sensitive(false);
        let title = label(
            &format!("Removing {}", plan.plan().manifest().app.name),
            "title-1",
        );
        self.detail.append(&title);
        let spinner = gtk::Spinner::new();
        spinner.start();
        spinner.set_halign(gtk::Align::Start);
        self.detail.append(&spinner);
        let stage = label("Preparing", "title-3");
        self.detail.append(&stage);
        self.detail.append(&label("Package operations can require authentication. Keep this window open until verification finishes.","dim-label"));
        enum Event {
            Progress(String),
            Done(Result<OperationResult>),
        }
        let (tx, rx) = async_channel::unbounded();
        let service = self.service.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(||service.execute(
                plan,
                |s| {
                    let _ = tx.send_blocking(Event::Progress(s.into()));
                },
                &Cancellation::default(),
            ))).unwrap_or_else(|_|Err("Operation worker stopped unexpectedly. Check the intent journal and package-manager state before retrying.".into()));
            let _ = tx.send_blocking(Event::Done(result));
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(event) = rx.recv().await {
                let Some(ui) = weak.upgrade() else {
                    return;
                };
                match event {
                    Event::Progress(s) => stage.set_label(&s),
                    Event::Done(result) => {
                        ui.busy.set(false);
                        ui.list.set_sensitive(true);
                        ui.clear();
                        match result {
                            Ok(result) => ui.result(&result),
                            Err(e) => {
                                ui.detail.append(
                                    &adw::StatusPage::builder()
                                        .title("Removal could not complete")
                                        .icon_name("dialog-warning-symbolic")
                                        .description(&e)
                                        .build(),
                                );
                            }
                        }
                        break;
                    }
                }
            }
        });
    }
    fn result(self: &Rc<Self>, result: &OperationResult) {
        let title = if result.package_removed && result.errors.is_empty() {
            "Application removed"
        } else if result.package_removed {
            "Application removed · cleanup incomplete"
        } else {
            "Removal not completed"
        };
        self.detail.append(
            &adw::StatusPage::builder()
                .title(title)
                .icon_name(if result.errors.is_empty() {
                    "emblem-ok-symbolic"
                } else {
                    "dialog-warning-symbolic"
                })
                .description(&result.app)
                .build(),
        );
        let group = adw::PreferencesGroup::new();
        group.add(&action_row(
            "Quarantined · recoverable",
            &format_size(Some(result.quarantined_bytes)),
        ));
        group.add(&action_row(
            "Freed disk space",
            "Unknown — shared storage and quarantine are not counted as freed",
        ));
        group.add(&action_row(
            "Preserved",
            "User documents, shared dependencies and unproven leftovers",
        ));
        self.detail.append(&group);
        let evidence = adw::ExpanderRow::builder()
            .title("View operation details")
            .use_markup(false)
            .build();
        for note in &result.details {
            evidence.add_row(&action_row("", note));
        }
        for id in &result.quarantined {
            evidence.add_row(&action_row("Quarantine record", id));
        }
        let evidence_list = gtk::ListBox::new();
        evidence_list.add_css_class("boxed-list");
        evidence_list.append(&evidence);
        self.detail.append(&evidence_list);
        for e in &result.errors {
            self.detail.append(&label(e, "warning"));
        }
        let done = gtk::Button::with_label("Done · Refresh Applications");
        done.add_css_class("suggested-action");
        let weak = Rc::downgrade(self);
        done.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.scan();
            }
        });
        self.detail.append(&done);
    }
    fn history(self: &Rc<Self>) {
        if self.busy.get() {
            return;
        }
        let dialog = adw::Dialog::builder()
            .title("History & Quarantine")
            .content_width(650)
            .content_height(620)
            .build();
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        let body = gtk::Box::new(gtk::Orientation::Vertical, 20);
        margins(&body, 24);
        body.append(&label("A transparent record", "title-1"));
        body.append(&label("Quarantined files remain recoverable. Restoring data does not reinstall package-manager applications.","dim-label"));
        toolbar.set_content(Some(
            &gtk::ScrolledWindow::builder()
                .vexpand(true)
                .child(&body)
                .build(),
        ));
        dialog.set_child(Some(&toolbar));
        dialog.present(Some(&self.window));
        let loading = gtk::Spinner::new();
        loading.start();
        body.append(&loading);
        let service = self.service.clone();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking((service.history(), files::quarantine_records(&service.home)));
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let received = rx.recv().await;
            body.remove(&loading);
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Ok((history, records)) = received else {
                body.append(&label("History worker stopped", "warning"));
                return;
            };
            match history {
                Ok(history) => {
                    let group = adw::PreferencesGroup::builder().title("Operations").build();
                    if history.is_empty() {
                        group.add(&action_row(
                            "No removals yet",
                            "Your operation history will appear here.",
                        ));
                    }
                    for entry in history.into_iter().take(100) {
                        group.add(&action_row(
                            &format!("{} · {}", entry.app, format_timestamp(entry.timestamp)),
                            &format!(
                                "{} · {} · Quarantined {}\n{}",
                                entry.backend.label(),
                                if entry.package_removed {
                                    "Removal verified"
                                } else {
                                    "Not removed"
                                },
                                format_size(Some(entry.quarantined_bytes)),
                                entry.errors.join("\n")
                            ),
                        ));
                    }
                    body.append(&group);
                }
                Err(e) => body.append(&label(&e, "warning")),
            }
            match records {
                Ok(records) => {
                    let group=adw::PreferencesGroup::builder().title("Quarantine").description("Retention: until manually managed. Automatic and permanent deletion are not enabled in this release.").build();
                    for record in records {
                        let row = action_row(
                            &record.original.display().to_string(),
                            &format_size(Some(record.tree.bytes)),
                        );
                        let restore = gtk::Button::builder()
                            .label("Restore")
                            .valign(gtk::Align::Center)
                            .build();
                        let home = ui.service.home.clone();
                        let weak = Rc::downgrade(&ui);
                        restore.connect_clicked(move |button| {
                            button.set_sensitive(false);
                            button.set_label("Restoring…");
                            let home = home.clone();
                            let id = record.id.clone();
                            let (tx, rx) = async_channel::bounded(1);
                            std::thread::spawn(move || {
                                let _ = tx.send_blocking(files::restore(&home, &id));
                            });
                            let weak = weak.clone();
                            let button = button.clone();
                            glib::spawn_future_local(async move {
                                match rx.recv().await {
                                    Ok(Ok(())) => button.set_label("Restored"),
                                    other => {
                                        button.set_sensitive(true);
                                        button.set_label("Restore");
                                        if let Some(ui) = weak.upgrade() {
                                            ui.message("Restore refused", &format!("{other:?}"));
                                        }
                                    }
                                }
                            });
                        });
                        row.add_suffix(&restore);
                        group.add(&row);
                    }
                    body.append(&group);
                }
                Err(e) => body.append(&label(&e, "warning")),
            }
        });
    }
    fn preferences(&self) {
        let dialog = adw::PreferencesDialog::builder()
            .title("Preferences")
            .build();
        let page = adw::PreferencesPage::new();
        let appearance = adw::PreferencesGroup::builder().title("Appearance").build();
        let row = adw::ComboRow::builder()
            .title("Color scheme")
            .model(&gtk::StringList::new(&["System", "Light", "Dark"]))
            .build();
        let manager = adw::StyleManager::default();
        row.set_selected(match manager.color_scheme() {
            adw::ColorScheme::ForceLight => 1,
            adw::ColorScheme::ForceDark => 2,
            _ => 0,
        });
        let service = self.service.clone();
        row.connect_selected_notify(move |row| {
            adw::StyleManager::default().set_color_scheme(match row.selected() {
                1 => adw::ColorScheme::ForceLight,
                2 => adw::ColorScheme::ForceDark,
                _ => adw::ColorScheme::Default,
            });
            let _ = service.save_appearance(row.selected());
        });
        appearance.add(&row);
        page.add(&appearance);
        let cleanup = adw::PreferencesGroup::builder()
            .title("Cleanup & safety")
            .build();
        cleanup.add(&action_row(
            "Always review before removal",
            "Confirmation and ownership validation cannot be disabled.",
        ));
        cleanup.add(&action_row(
            "Personal data",
            "Preserved by default. Complete Removal offers only proven candidates.",
        ));
        cleanup.add(&action_row(
            "Quarantine retention",
            "Until manually managed. Automatic and permanent deletion are not enabled.",
        ));
        page.add(&cleanup);
        let images = adw::PreferencesGroup::builder()
            .title("AppImage discovery")
            .build();
        images.add(&action_row(
            "Non-recursive locations",
            "~/Applications\n~/.local/bin\n~/Downloads",
        ));
        images.add(&action_row(
            "Custom locations",
            "Not enabled in this conservative first release.",
        ));
        page.add(&images);
        dialog.add(&page);
        dialog.present(Some(&self.window));
    }
    fn about(&self) {
        let about=adw::AboutDialog::builder().application_name("Cleanly").application_icon(APP_ID).version("0.1.0 · Preview").developer_name("Cleanly contributors").license_type(gtk::License::Gpl30).comments("Inspect first. Remove with confidence.\n\nA conservative Linux application inspector and uninstaller. This is an early tested vertical slice, not a production-certified release.").build();
        about.present(Some(&self.window));
    }
}

// Opt-in developer smoke test. Reads real discovery; never confirms or invokes removal.
fn smoke_test(ui: Rc<Ui>) {
    let weak = Rc::downgrade(&ui);
    glib::timeout_add_local_once(std::time::Duration::from_secs(4), move || {
        if let Some(ui) = weak.upgrade() {
            let app = ui
                .apps
                .borrow()
                .iter()
                .find(|a| a.name == "Firefox")
                .cloned();
            if let Some(app) = app {
                ui.select(app);
            }
        }
    });
    glib::timeout_add_local_once(std::time::Duration::from_secs(12), move || {
        let paintable = gtk::WidgetPaintable::new(Some(&ui.window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(
            &snapshot,
            ui.window.width() as f64,
            ui.window.height() as f64,
        );
        if let (Some(node), Some(renderer)) = (snapshot.to_node(), ui.window.renderer()) {
            let texture = renderer.render_texture(&node, None);
            match texture.save_to_png(format!(
                "/tmp/cleanly-{}.png",
                std::env::var("CLEANLY_SMOKE_VIEW").unwrap_or_else(|_| "smoke".into())
            )) {
                Ok(()) => eprintln!(
                    "SMOKE: screenshot saved; {} apps; inspection loaded: {}",
                    ui.apps.borrow().len(),
                    ui.manifest.borrow().is_some()
                ),
                Err(e) => eprintln!("SMOKE screenshot failed: {e}"),
            }
        }
        if let Some(app) = ui.window.application() {
            app.quit();
        }
    });
}

fn format_timestamp(value: u64) -> String {
    if value == 0 {
        return "Date unknown".into();
    }
    glib::DateTime::from_unix_local(value as i64)
        .ok()
        .and_then(|d| d.format("%b %e, %Y · %H:%M").ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Date unknown".into())
}
