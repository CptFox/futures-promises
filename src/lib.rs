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
    enum StreamState {
        NotReady,
        Ready,
        Closed,
    }

    /// This `futures::Stream` implementation will be notified whenever a `WatchedVariableAccessor` is dropped
    /// If the accessor was mutably derefenced, then a clone of the value after dropping will be sent upon polling
    pub struct VariableWatcher<T> {
        task: Arc<AtomicTask>,
        content: Arc<Mutex<(T, StreamState)>>,
    }

    impl<T> VariableWatcher<T> {
        /// Triggers a new poll without changes to the watched variable
        pub fn trigger(&self) {
            (*self.content.lock().unwrap()).1 = StreamState::Ready;
            self.task.notify();
        }
    }

    impl<T> Stream for VariableWatcher<T>
    where
        T: Clone,
    {
        type Item = T;
        type Error = Infallible;
        fn poll(&mut self) -> Poll<Option<<Self as Stream>::Item>, <Self as Stream>::Error> {
            let mut guard = self.content.lock().unwrap();
            match (guard.1).clone() {
                StreamState::NotReady => {
                    self.task.register();
                    Ok(Async::NotReady)
                }
                StreamState::Closed => Ok(Async::Ready(None)),
                StreamState::Ready => {
                    (*guard).1 = StreamState::NotReady;
                    Ok(Async::Ready(Some(guard.0.clone())))
                }
            }
        }
    }

    /// A watched variable. Behaves similarly to a mutex, except that watchers obtained from its
    /// `get_watcher()` method will be notified upon mutable dereferencing.
    #[derive(Clone)]
    pub struct WatchedVariable<T> {
        task: Arc<AtomicTask>,
        content: Arc<Mutex<(T, StreamState)>>,
    }

    impl<T> WatchedVariable<T> {
        /// Constructs a `WatchedVariable` from `value`. This initialisation value will be returned by the first `poll()`
        /// on its watchers unless altered before the watchers are started
        pub fn from(value: T) -> WatchedVariable<T> {
            WatchedVariable {
                task: Arc::new(AtomicTask::new()),
                content: Arc::new(Mutex::new((value, StreamState::Ready))),
            }
        }

        pub fn get_watcher(&self) -> VariableWatcher<T> {
            VariableWatcher {
                task: self.task.clone(),
                content: self.content.clone(),
            }
        }

        /// Similar to Mutex::lock()
        pub fn lock(&self) -> WatchedVariableAccessor<T> {
            WatchedVariableAccessor {
                task: self.task.clone(),
                content: self.content.lock().unwrap(),
            }
        }
    }

    impl<T> Drop for WatchedVariable<T> {
        fn drop(&mut self) {
            let mut guard = self.content.lock().unwrap();
            guard.1 = StreamState::Closed;
            self.task.notify();
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
    #[derive(Clone)]
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
            self.content.lock().unwrap().set(Some(value));
            *self.state.lock().unwrap() = PromiseState::Resolved;
            self.task.notify();
        }

        pub fn reject(&self, message: String) {
            *self.state.lock().unwrap() = PromiseState::Rejected(message);
            self.task.notify();
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
            *self.state.lock().unwrap() = PromiseState::Rejected("Promise Dropped".into());
            self.task.notify();
        }
    }

    /// The "receiver": a `Future` used to watch a `Promise`
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
