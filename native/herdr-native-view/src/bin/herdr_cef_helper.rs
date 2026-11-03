//! Chromium helper process for Herdr Preview Dev (GPU, renderer, plugin,
//! alerts and utility processes). Copied into each
//! `Contents/Frameworks/<exe> Helper*.app` by scripts/build.
fn main() {
    #[cfg(target_os = "macos")]
    {
        let exe = std::env::current_exe().expect("helper path");
        let loader = cef::library_loader::LibraryLoader::new(&exe, true);
        if !loader.load() {
            eprintln!("herdr-cef-helper: cannot load the Chromium framework");
            std::process::exit(1);
        }
        let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
        let args = cef::args::Args::new();
        let code = cef::execute_process(Some(args.as_main_args()), None, std::ptr::null_mut());
        std::process::exit(code.max(0));
    }
}
