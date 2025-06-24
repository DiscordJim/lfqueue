# lfqueue
A lock-free queue for asynchronous &amp; synchronous code.

# Loom
```bash
$ LOOM_MAX_PREEMPTIONS=3 RUSTFLAGS="--cfg loom" cargo test loom_ --release
```

# Fuzzing

## Running Fuzz Tests
```bash
$ RUSTFLAGS="--cfg fuzzing" cargo +nightly fuzz run initfull
```

## Tests
- `initfull` tests a special method for `ScqRing` which is the core component of `ScqQueue` and thus `LcsqQueue` where the queue is initialized to a full state.


# Miri
```bash
$ rustup +nightly component add miri
$ cargo +nightly miri test
```
