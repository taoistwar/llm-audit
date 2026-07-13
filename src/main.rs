#[tokio::main]
async fn main() {
    // Load .env file if it exists, but don't fail if it doesn't
    match dotenvy::dotenv() {
        Ok(path) => {
            println!("[ENV] Loaded .env from: {}", path.display());
        }
        Err(_) => {
            println!("[ENV] No .env file found, using environment variables");
        }
    }
    llm_audit::start_server().await;
}
