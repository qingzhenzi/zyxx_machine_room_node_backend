#[tokio::main]
async fn main() -> anyhow::Result<()> {
    qe_api::run().await
}
