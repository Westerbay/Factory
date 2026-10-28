use std::io::{self, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = serde_json::to_string_pretty(&factory_api::openapi::document())?;
    writeln!(io::stdout().lock(), "{output}")?;
    Ok(())
}
