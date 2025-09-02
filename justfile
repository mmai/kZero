dockerbuild:
  docker build -t mmai/kzero-trictrac:latest .
dockerpush:
  docker push mmai/kzero-trictrac:latest
build:
  cargo build --release --manifest-path rust/Cargo.toml -p kz-selfplay
trainerstart:
  PYTHON_PATH=./python python python/main/loop_main_alpha.py --game trictrac --data-path data/loop --new
selfplaystart:
  ./rust/target/release/selfplay --port 63105
