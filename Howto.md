# Howto

1. Compile client

```sh
cargo build --release --manifest-path rust/Cargo.toml -p kz-selfplay
```

2. Start training server

```sh
python python/main/loop_main_alpha.py --game trictrac --data-path data/loop --new
```

3. Start self-play server

```sh
./rust/target/release/selfplay --port 63105
```
