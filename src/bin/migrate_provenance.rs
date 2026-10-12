//! Compatibility launcher for the measurements-owned provenance migration command.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    autoeq_measurements::provenance_migration_cli::run()
}
