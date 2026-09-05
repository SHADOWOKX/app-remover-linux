use cleanly_service::privileged::{Request, execute};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let result = (|| {
        if args.len() != 2 {
            return Err("Expected one structured request".to_string());
        }
        let request = Request::parse(&args[1])?;
        let status = std::fs::read_to_string("/proc/self/status").map_err(|e| e.to_string())?;
        if status
            .lines()
            .find(|l| l.starts_with("Uid:"))
            .and_then(|l| l.split_whitespace().nth(2))
            != Some("0")
        {
            return Err("Helper requires polkit authorization".into());
        }
        // SAFETY: this helper is single-threaded and has not created any worker threads.
        for (key, _) in std::env::vars_os() {
            unsafe {
                std::env::remove_var(key);
            }
        }
        unsafe {
            std::env::set_var("HOME", "/root");
            std::env::set_var("PATH", "/usr/sbin:/usr/bin:/sbin:/bin");
            std::env::set_var("LC_ALL", "C");
        }
        std::env::set_current_dir("/").map_err(|e| e.to_string())?;
        execute(request)
    })();
    if let Err(e) = result {
        eprintln!("Cleanly refused or could not complete removal: {e}");
        std::process::exit(1);
    }
}
