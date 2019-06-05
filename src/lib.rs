extern crate futures;

/// A futures implementation of watched variables.
pub mod watched_variables {
    use futures::task::AtomicTask;
    use futures::{Async, Poll, Stream};

    use std::convert::Infallible;
    use std::ops::Deref;
    use std::ops::DerefMut;
    use std::sync::MutexGuard;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    pub enum StreamState {
        NotReady,
        Ready,
        Closed,
    }

    /// This `futures::Stream` implementation will be notified whenever a `WatchedVariableAccessor` is dropped
    /// If the accessor was mutably derefenced, then a clone of the value after dropping will be sent upon polling
    ///
    /// It implements Stream, where each frame will be a clone of its content (an Arc on the variable)
    #[derive(Clone)]
    pub struct VariableWatcher<T> {
        pub task: Arc<AtomicTask>,
        pub content: Arc<Mutex<(T, StreamState)>>,
    }

    impl<T> Stream for VariableWatcher<T> {
        type Item = Arc<Mutex<(T, StreamState)>>;
        type Error = Infallible;
        fn poll(&mut self) -> Poll<Option<<Self as Stream>::Item>, <Self as Stream>::Error> {
            self.task.register();
            let mut guard = self.content.lock().unwrap();
            match (guard.1).clone() {
                StreamState::NotReady => Ok(Async::NotReady),
                StreamState::Closed => Ok(Async::Ready(None)),
                StreamState::Ready => {
                    (*guard).1 = StreamState::NotReady;
                    Ok(Async::Ready(Some(self.content.clone())))
                }
            }
        }
    }

    /// A watched variable. Behaves similarly to a mutex, except that watchers obtained from its
    /// `get_watcher()` method will be notified upon mutable dereferencing.
    pub struct WatchedVariable<T> {
        task: Arc<AtomicTask>,
        content: Arc<Mutex<(T, StreamState)>>,
        counter: Arc<Mutex<u32>>,
    }

    impl<T> Clone for WatchedVariable<T> {
        fn clone(&self) -> Self {
            {
                *self.counter.lock().unwrap() += 1;
            }
            WatchedVariable {
                task: self.task.clone(),
                content: self.content.clone(),
                counter: self.counter.clone(),
            }
        }
    }

    impl<T> WatchedVariable<T> {
        /// Constructs a `WatchedVariable` from `value`. This initialisation value will be returned by the first `poll()`
        /// on its watchers unless altered before the watchers are started
        pub fn from(value: T) -> WatchedVariable<T> {
            WatchedVariable {
                task: Arc::new(AtomicTask::new()),
                content: Arc::new(Mutex::new((value, StreamState::Ready))),
                counter: Arc::new(Mutex::new(1)),
            }
        }

        pub fn get_watcher(&self) -> VariableWatcher<T> {
            VariableWatcher {
                task: self.task.clone(),
                content: self.content.clone(),
            }
        }

        /// Similar to Mutex::lock(), but the provided Accessor will trigger a `poll`
        /// upon `drop`, which will resolve to Ready if the accessor was accessed mutably.
        pub fn lock(&self) -> WatchedVariableAccessor<T> {
            WatchedVariableAccessor {
                task: self.task.clone(),
                content: self.content.lock().unwrap(),
            }
        }

        /// Allows to force ready upon the watcher.
        pub fn force_ready(&self) {
            self.content.lock().unwrap().1 = StreamState::Ready;
            self.task.notify();
        }
    }

    impl<T> Drop for WatchedVariable<T> {
        fn drop(&mut self) {
            let mut guard = self.counter.lock().unwrap();
            *guard -= 1;
            if *guard <= 0 {
                let mut guard = self.content.lock().unwrap();
                guard.1 = StreamState::Closed;
                self.task.notify();
            }
        }
    }

    /// Similar to a MutexGuard, but dropping it will also notify watchers associated with it.
    pub struct WatchedVariableAccessor<'a, T> {
        task: Arc<AtomicTask>,
        content: MutexGuard<'a, (T, StreamState)>,
    }

    impl<'a, T> Drop for WatchedVariableAccessor<'a, T> {
        fn drop(&mut self) {
            self.task.notify();
        }
    }

    impl<'a, T> Deref for WatchedVariableAccessor<'a, T> {
        type Target = T;
        fn deref(&self) -> &<Self as Deref>::Target {
            return &self.content.0;
        }
    }

    impl<'a, T> DerefMut for WatchedVariableAccessor<'a, T> {
        fn deref_mut(&mut self) -> &mut <Self as Deref>::Target {
            self.content.1 = StreamState::Ready;
            return &mut self.content.0;
        }
    }
}

/// A futures implementation of JS-like Promises.
pub mod promises {
    use std::cell::Cell;
    use std::sync::{Arc, Mutex};

    use futures::task::AtomicTask;
    use futures::{Async, Future, Poll};

    #[derive(Clone)]
    enum PromiseState {
        NotReady,
        Resolved,
        Rejected(String),
    }

    /// The "sender" side of a Promise
    pub struct Promise<T> {
        content: Arc<Mutex<Cell<Option<T>>>>,
        state: Arc<Mutex<PromiseState>>,
        task: Arc<AtomicTask>,
    }

    impl<T> Promise<T> {
        pub fn new() -> Self {
            Promise {
                content: Arc::new(Mutex::new(Cell::new(None))),
                state: Arc::new(Mutex::new(PromiseState::NotReady)),
                task: Arc::new(AtomicTask::new()),
            }
        }

        pub fn resolve(&self, value: T) {
            let mut guard = self.state.lock().unwrap();
            match (*guard).clone() {
                PromiseState::NotReady => {
                    self.content.lock().unwrap().set(Some(value));
                    *guard = PromiseState::Resolved;
                    self.task.notify();
                }
                _ => {
                    panic!("Attempt to resolve an already finished promise");
                }
            }
        }

        pub fn reject(&self, message: String) {
            let mut guard = self.state.lock().unwrap();
            match (*guard).clone() {
                PromiseState::NotReady => {
                    *guard = PromiseState::Rejected(message);
                    self.task.notify();
                }
                _ => {
                    panic!("Attempt to reject an already finished promise");
                }
            }
        }

        pub fn get_handle(&self) -> PromiseHandle<T> {
            PromiseHandle {
                content: self.content.clone(),
                state: self.state.clone(),
                task: self.task.clone(),
            }
        }
    }

    impl<T> Drop for Promise<T> {
        fn drop(&mut self) {
            let mut guard = self.state.lock().unwrap();
            match (*guard).clone() {
                PromiseState::NotReady => {
                    *guard = PromiseState::Rejected("Promise Dropped".into());
                    self.task.notify();
                }
                _ => {}
            }
        }
    }

    /// The "receiver": a `Future` used to watch a `Promise`
    #[derive(Clone)]
    pub struct PromiseHandle<T> {
        content: Arc<Mutex<Cell<Option<T>>>>,
        state: Arc<Mutex<PromiseState>>,
        task: Arc<AtomicTask>,
    }

    impl<T> Future for PromiseHandle<T> {
        type Item = T;
        type Error = String;

        fn poll(&mut self) -> Poll<<Self as Future>::Item, <Self as Future>::Error> {
            match *self.state.lock().unwrap() {
                PromiseState::NotReady => {
                    self.task.register();
                    Ok(Async::NotReady)
                }
                PromiseState::Rejected(ref reason) => Err(reason.clone()),
                PromiseState::Resolved => match self.content.lock().unwrap().take() {
                    Some(value) => Ok(Async::Ready(value)),
                    None => Err("Promise resolved but value was None".into()),
                },
            }
        }
    }
}
