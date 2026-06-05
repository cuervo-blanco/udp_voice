fn main() -> Result<(), Box<dyn std::error::Error>> {
    selflib::peer::run_from_env()
}
