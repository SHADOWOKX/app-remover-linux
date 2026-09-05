use cleanly_core::*;
use cleanly_service::Service;
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .expect("HOME required");
    let service = Service::new(home);
    let cancel = Cancellation::default();
    if args.get(1).map(String::as_str) == Some("inspect") {
        let key = args.get(2).expect("inspect requires exact key from list");
        let found = std::sync::Mutex::new(None);
        let kind = match key.split(':').next() {
            Some("APT") => Backend::Apt,
            Some("Flatpak") => Backend::Flatpak,
            Some("Snap") => Backend::Snap,
            Some("AppImage") => Backend::AppImage,
            _ => Backend::Manual,
        };
        if let Ok(apps) = service.backend(kind).and_then(|b| b.discover(&cancel)) {
            for app in apps {
                if &app.key() == key {
                    *found.lock().unwrap() = Some(app);
                }
            }
        }
        let result = found
            .into_inner()
            .unwrap()
            .ok_or("Application not found".to_string())
            .and_then(|app| service.inspect(&app, &cancel));
        match result {
            Ok(manifest) => println!("{}", toml::to_string_pretty(&manifest).unwrap()),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    } else {
        service.discover(
            |d| {
                if let Some(e) = d.error {
                    eprintln!("{} support unavailable: {e}", d.backend.label());
                }
                for app in d.apps {
                    println!(
                        "{}\t{}\t{}\t{}",
                        app.key(),
                        app.name,
                        app.version,
                        format_size(app.size)
                    );
                }
            },
            &cancel,
        );
    }
}
