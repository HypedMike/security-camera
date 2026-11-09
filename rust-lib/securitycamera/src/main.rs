fn main() {
    if let Err(e) = security_camera::app::run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
