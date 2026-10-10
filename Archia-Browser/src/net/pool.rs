use std::sync::{Arc, Mutex};

use super::{Request, Response, Transport, TransportError};

pub trait Connection: Send + Sync {
    fn send(&self, request: &Request) -> Result<Response, TransportError>;
}

#[derive(Default)]
pub struct ConnectionPool {
    connections: Mutex<Vec<Arc<dyn Connection>>>,
}

impl ConnectionPool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&self, connection: Arc<dyn Connection>) {
        self.connections
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(connection);
    }

    pub fn len(&self) -> usize {
        self.connections
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.connections
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }
}

pub struct PooledTransport {
    pool: Arc<ConnectionPool>,
}

impl PooledTransport {
    pub fn new(pool: Arc<ConnectionPool>) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &Arc<ConnectionPool> {
        &self.pool
    }
}

impl Transport for PooledTransport {
    fn send(&self, request: &Request) -> Result<Response, TransportError> {
        // The pool owns only a Vec of Arc handles. Recovering its poisoned
        // mutex is safe because Vec preserves its invariants across unwinding
        // and no caller can mutate the vector except through this lock.
        let connection = self
            .pool
            .connections
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .last()
            .cloned()
            .ok_or(TransportError::ConnectionFailed)?;

        connection.send(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::Url;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    struct OkConnection;

    impl Connection for OkConnection {
        fn send(&self, _: &Request) -> Result<Response, TransportError> {
            Ok(Response::new(200))
        }
    }

    #[test]
    fn poisoned_pool_mutex_does_not_turn_into_a_browser_panic() {
        let pool = Arc::new(ConnectionPool::new());
        let poisoned = catch_unwind(AssertUnwindSafe(|| {
            let _guard = pool.connections.lock().unwrap();
            panic!("simulate interruption while holding pool lock");
        }));
        assert!(poisoned.is_err());

        assert!(pool.is_empty());
        assert_eq!(pool.len(), 0);
        pool.add(Arc::new(OkConnection));

        let request = Request::new(Url::parse("https://example.org/").unwrap());
        let response = PooledTransport::new(Arc::clone(&pool))
            .send(&request)
            .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(pool.len(), 1);
    }
}
