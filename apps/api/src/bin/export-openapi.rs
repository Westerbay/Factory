use std::io::{self, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let document = serde_json::to_value(factory_api::openapi::document())?;
    let output = serde_json::to_string_pretty(&document)?;
    writeln!(io::stdout().lock(), "{output}")?;
    Ok(())
}
