use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::task::AbortHandle;

pub struct AppState {
    pub session: Mutex<Option<AbortHandle>>,
    pub panic_mode: Arc<Mutex<bool>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            session: Mutex::new(None),
            panic_mode: Arc::new(Mutex::new(false)),
        }
    }
}
