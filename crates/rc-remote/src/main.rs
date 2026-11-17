#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rc_remote::serve(tokio::io::stdin(), tokio::io::stdout()).await
}
