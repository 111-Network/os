fn main() {
    std::process::exit(node111::cli::main_entry(std::env::args().skip(1).collect()));
}
