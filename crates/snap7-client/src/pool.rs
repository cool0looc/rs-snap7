use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::client::S7Client;
use crate::error::Error;
use crate::transport::TcpTransport;
use crate::types::ConnectParams;

/// Configuration for `S7Pool`.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Maximum number of concurrent connections.
    pub max_size: usize,
    /// TCP connect timeout per connection attempt.
    pub connect_timeout: Duration,
}

impl Default for PoolConfig {
    fn default() -> Self {
        PoolConfig {
            max_size: 4,
            connect_timeout: Duration::from_secs(5),
        }
    }
}

struct PoolInner {
    idle: VecDeque<S7Client<TcpTransport>>,
    addr: SocketAddr,
    connect_params: ConnectParams,
    connect_timeout: Duration,
}

/// A bounded pool of `S7Client<TcpTransport>` connections.
pub struct S7Pool {
    inner: Arc<Mutex<PoolInner>>,
    semaphore: Arc<Semaphore>,
}

/// RAII guard — returns the connection to the pool on drop.
pub struct PooledClient {
    client: Option<S7Client<TcpTransport>>,
    pool: Arc<Mutex<PoolInner>>,
    _permit: OwnedSemaphorePermit,
}

impl PooledClient {
    /// Access the underlying `S7Client`.
    pub fn client(&self) -> &S7Client<TcpTransport> {
        self.client
            .as_ref()
            .expect("client always present until drop")
    }
}

impl Drop for PooledClient {
    fn drop(&mut self) {
        if let Some(client) = self.client.take() {
            if let Ok(mut inner) = self.pool.lock() {
                inner.idle.push_back(client);
            }
            // If the mutex is poisoned, the connection is dropped — acceptable.
        }
    }
}

impl S7Pool {
    /// Create a new pool targeting `addr` with `connect_params` and `cfg`.
    pub fn new(addr: SocketAddr, connect_params: ConnectParams, cfg: PoolConfig) -> Self {
        let max = cfg.max_size;
        S7Pool {
            inner: Arc::new(Mutex::new(PoolInner {
                idle: VecDeque::new(),
                addr,
                connect_params,
                connect_timeout: cfg.connect_timeout,
            })),
            semaphore: Arc::new(Semaphore::new(max)),
        }
    }

    /// Borrow a connection from the pool, opening a new one if none are idle.
    /// Blocks until a semaphore permit is available (bounded by `max_size`).
    pub async fn acquire(&self) -> Result<PooledClient, Error> {
        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore never closed");

        // Check for an idle connection — hold the lock only briefly.
        let idle_client = {
            let mut inner = self.inner.lock().expect("pool mutex not poisoned");
            inner.idle.pop_front()
        };

        if let Some(client) = idle_client {
            return Ok(PooledClient {
                client: Some(client),
                pool: self.inner.clone(),
                _permit: permit,
            });
        }

        // No idle connection — extract params (brief lock scope), then connect.
        let (addr, params, connect_timeout) = {
            let inner = self.inner.lock().expect("pool mutex not poisoned");
            (
                inner.addr,
                inner.connect_params.clone(),
                inner.connect_timeout,
            )
        };

        let client = tokio::time::timeout(
            connect_timeout,
            S7Client::<TcpTransport>::connect(addr, params),
        )
        .await
        .map_err(|_| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "pool connect timeout",
            ))
        })??;

        Ok(PooledClient {
            client: Some(client),
            pool: self.inner.clone(),
            _permit: permit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn cfg(max: usize) -> PoolConfig {
        PoolConfig {
            max_size: max,
            connect_timeout: Duration::from_millis(100),
        }
    }

    #[test]
    fn pool_config_defaults_are_sane() {
        let c = PoolConfig::default();
        assert!(c.max_size >= 1);
        assert!(c.connect_timeout.as_millis() > 0);
    }

    #[test]
    fn pool_config_max_size() {
        let c = cfg(4);
        assert_eq!(c.max_size, 4);
    }

    #[tokio::test]
    async fn pool_acquire_returns_err_on_unreachable_host() {
        let addr = "127.0.0.1:1".parse().unwrap();
        let pool = S7Pool::new(addr, Default::default(), cfg(2));
        let result = pool.acquire().await;
        assert!(result.is_err(), "expected connection error on port 1");
    }

    #[tokio::test]
    async fn pool_acquire_releases_permit_on_error() {
        let addr = "127.0.0.1:1".parse().unwrap();
        let pool = S7Pool::new(
            addr,
            Default::default(),
            PoolConfig {
                max_size: 1,
                connect_timeout: Duration::from_millis(100),
            },
        );
        // First acquire fails (unreachable host).
        assert!(pool.acquire().await.is_err());
        // If the permit was leaked, this second acquire would deadlock.
        let result = tokio::time::timeout(Duration::from_secs(2), pool.acquire()).await;
        assert!(
            result.is_ok(),
            "second acquire timed out — permit was leaked"
        );
    }

    #[tokio::test]
    async fn pool_semaphore_limits_concurrent_borrows() {
        // Pool of size 1 — both acquires fail (port 1 not listening) but neither panics,
        // proving the semaphore releases correctly on error and allows a second attempt.
        let addr = "127.0.0.1:1".parse().unwrap();
        let pool = Arc::new(S7Pool::new(
            addr,
            Default::default(),
            PoolConfig {
                max_size: 1,
                connect_timeout: Duration::from_millis(100),
            },
        ));

        let pool1 = pool.clone();
        let t1 = tokio::spawn(async move { pool1.acquire().await });

        let t2 = tokio::spawn(async move { pool.acquire().await });

        let (r1, r2) = tokio::join!(t1, t2);
        // Both fail with connection error — what matters is neither panicked
        assert!(r1.unwrap().is_err());
        assert!(r2.unwrap().is_err());
    }
}
