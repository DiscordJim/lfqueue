# lfqueue
A lock-free queue for asynchronous &amp; synchronous code.

# Loom
```bash
$ LOOM_MAX_PREEMPTIONS=3 RUSTFLAGS="--cfg loom" cargo test loom_ --release
```

# Fuzzing
```bash
$ RUSTFLAGS="--cfg fuzzing" cargo +nightly fuzz run initfull
```

# Miri
```bash
$ rustup +nightly component add miri
$ cargo +nightly miri test
```
