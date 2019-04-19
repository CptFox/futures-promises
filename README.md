# Promises and Watched Variables for Futures

## Promises
Promises are future values.
They will only resolve once.
This crate gives an implementation of Promises to work with Futures and Tokio

## Watched Variables
Watched variables are variables that will emit a signal upon being altered.

This crate offers an RAII implementation of watched variables, to work with Futures and Tokio.

In this case, the signal is a `futures::Stream` that will return a clone of the new value of the watched variable
whenever it is mutated.

This crate will notify the `VariableWatched` obtained from a `WatchedVariable` whenever one of its accessors (obtained through `lock()`)
is dropped. If said accessor has been mutably derefenced, then the `WatchedVariable` will be considered to have been mutated.