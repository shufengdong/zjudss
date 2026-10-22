fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = backend::contract_manifest()?;
    println!("{}", serde_json::to_string_pretty(&manifest)?);
    Ok(())
}
