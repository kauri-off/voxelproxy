use tokio::sync::Mutex;
use tokio::task::AbortHandle;

pub struct AppState {
    pub session: Mutex<Option<AbortHandle>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            session: Mutex::new(None),
        }
    }
}
