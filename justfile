dockerbuild:
  docker build -t mmai/kzero-trictrac:latest .
dockerpush:
  docker push mmai/kzero-trictrac:latest
build:
  cargo build --release --manifest-path rust/Cargo.toml -p kz-selfplay
trainerstart:
  PYTHONPATH=./python:$PYTHONPATH python python/main/loop_main_alpha.py
selfplaystart:
  ./rust/target/release/selfplay --port 63105
