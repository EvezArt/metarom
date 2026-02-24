fn main() {
    if let Err(e) = ucf_planner::cli::run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
