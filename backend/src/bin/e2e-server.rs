#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("e2e server 启动失败: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    zjudss::testing::run_e2e_server(zjudss::testing::E2eConfig::from_env()?).await
}
