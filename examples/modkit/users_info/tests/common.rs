use anyhow::Result;
use std::time::Duration;
use testcontainers::{runners::AsyncRunner, ImageExt};

pub struct DbUnderTest {
    pub url: String,
    #[allow(dead_code, clippy::type_complexity)]
    _cleanup: Option<Box<dyn FnOnce() + Send + Sync>>,
}

pub async fn bring_up_postgres() -> Result<DbUnderTest> {
    use testcontainers::ContainerRequest;
    use testcontainers_modules::postgres::Postgres;

    let postgres_image = Postgres::default();
    let container_request = ContainerRequest::from(postgres_image)
        .with_env_var("POSTGRES_PASSWORD", "pass")
        .with_env_var("POSTGRES_USER", "user")
        .with_env_var("POSTGRES_DB", "app");

    let container = container_request.start().await?;
    let port = container.get_host_port_ipv4(5432).await?;
    wait_for_tcp("127.0.0.1", port, Duration::from_secs(20)).await?;

    Ok(DbUnderTest {
        url: format!("postgres://user:pass@127.0.0.1:{port}/app"),
        _cleanup: Some(Box::new(move || drop(container))),
    })
}

async fn wait_for_tcp(host: &str, port: u16, timeout: Duration) -> Result<()> {
    use tokio::{
        net::TcpStream,
        time::{sleep, Instant},
    };
    let deadline = Instant::now() + timeout;
    loop {
        if TcpStream::connect((host, port)).await.is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            anyhow::bail!("Timeout waiting for {host}:{port}");
        }
        sleep(Duration::from_millis(200)).await;
    }
}
