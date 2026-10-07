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
            .expect("connection pool poisoned")
            .push(connection);
    }

    pub fn len(&self) -> usize {
        self.connections
            .lock()
            .expect("connection pool poisoned")
            .len()
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
        let connection = self
            .pool
            .connections
            .lock()
            .expect("connection pool poisoned")
            .last()
            .cloned()
            .ok_or(TransportError::ConnectionFailed)?;

        connection.send(request)
    }
}
