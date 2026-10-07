use std::collections::VecDeque;

pub type Task = Box<dyn FnOnce() + Send + 'static>;

#[derive(Default)]
pub struct EventLoop {
    queue: VecDeque<Task>,
}

impl EventLoop {
    pub fn new() -> Self { Self::default() }

    pub fn spawn<F>(&mut self, task: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.queue.push_back(Box::new(task));
    }

    pub fn pending(&self) -> usize { self.queue.len() }

    pub fn run_one(&mut self) -> bool {
        let Some(task) = self.queue.pop_front() else { return false };
        task();
        true
    }

    pub fn run_until_idle(&mut self) {
        while self.run_one() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn tasks_run_in_fifo_order() {
        let values = Arc::new(Mutex::new(Vec::new()));
        let mut event_loop = EventLoop::new();
        for value in 0..3 {
            let values = Arc::clone(&values);
            event_loop.spawn(move || values.lock().unwrap().push(value));
        }
        event_loop.run_until_idle();
        assert_eq!(*values.lock().unwrap(), vec![0, 1, 2]);
    }
}
