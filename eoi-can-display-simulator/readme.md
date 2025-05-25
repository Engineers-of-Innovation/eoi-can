# Simulator

run `cargo run` to run simulator

or `EG_SIMULATOR_DUMP=screenshot.png cargo run` to generate screenshot

## Flamegraph - Performance / CPU usage

To get an idea of the performance of the code, you can use `flamegraph`, more info [here](https://github.com/flamegraph-rs/flamegraph).

```bash
CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph
```

you might need to lower some permissions (temporary)

```bash
echo 0 | sudo tee /proc/sys/kernel/kptr_restrict
echo -1 | sudo tee /proc/sys/kernel/perf_event_paranoid
```
