//! Developer integration check. Packages an already-built runtime without a UI.
//! Usage: cargo run --example export-app -- /outside/config.json /path/Runtime.app /outside/Output.app
#[path = "../src/app_export.rs"]
#[allow(dead_code)]
mod app_export;
#[path = "../src/configuration.rs"]
#[allow(dead_code)]
mod configuration;

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 {
        eprintln!("Expected configuration, prebuilt template and output .app paths");
        std::process::exit(2);
    }
    let result =
        configuration::AppConfig::read(std::path::Path::new(&args[0])).and_then(|config| {
            app_export::export(
                config,
                std::path::Path::new(&args[1]),
                std::path::Path::new(&args[2]),
            )
        });
    match result {
        Ok(app) => println!("{}", serde_json::to_string(&app).unwrap()),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
