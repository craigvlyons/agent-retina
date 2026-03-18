use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Debug)]
pub struct ExecutionConfig {
    pub max_steps: usize,
    pub control: Option<ExecutionControlHandle>,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            max_steps: 50,
            control: None,
        }
    }
}

#[derive(Debug)]
struct ExecutionControlState {
    cancel_requested: AtomicBool,
    operator_guidance: Mutex<Option<String>>,
}

#[derive(Clone, Debug)]
pub struct ExecutionControl {
    state: Arc<ExecutionControlState>,
}

#[derive(Clone, Debug)]
pub struct ExecutionControlHandle {
    state: Arc<ExecutionControlState>,
}

impl ExecutionControl {
    pub fn new() -> Self {
        Self {
            state: Arc::new(ExecutionControlState {
                cancel_requested: AtomicBool::new(false),
                operator_guidance: Mutex::new(None),
            }),
        }
    }

    pub fn handle(&self) -> ExecutionControlHandle {
        ExecutionControlHandle {
            state: Arc::clone(&self.state),
        }
    }
}

impl Default for ExecutionControl {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutionControlHandle {
    pub fn request_cancel(&self) {
        self.state.cancel_requested.store(true, Ordering::Relaxed);
    }

    pub fn is_cancel_requested(&self) -> bool {
        self.state.cancel_requested.load(Ordering::Relaxed)
    }

    pub fn queue_guidance(&self, guidance: impl Into<String>) {
        *recover_mutex(&self.state.operator_guidance) = Some(guidance.into());
    }

    pub fn take_guidance(&self) -> Option<String> {
        recover_mutex(&self.state.operator_guidance).take()
    }
}

fn recover_mutex<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
