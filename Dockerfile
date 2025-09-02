# Start from PyTorch with CUDA + cuDNN + full CUDA toolkit (devel)
FROM pytorch/pytorch:2.2.2-cuda12.1-cudnn8-devel

ENV DEBIAN_FRONTEND=noninteractive

# Base packages for building
RUN apt-get update && apt-get install -y \
    git curl build-essential pkg-config cmake \
    libssl-dev ca-certificates tmux nano \
    && rm -rf /var/lib/apt/lists/*

# CUDA env (toolkit available at /usr/local/cuda in *-devel images)
ENV CUDA_HOME=/usr/local/cuda
ENV CUDA_PATH=/usr/local/cuda
ENV LD_LIBRARY_PATH=/usr/local/cuda/lib64:${LD_LIBRARY_PATH}
ENV PATH=/usr/local/cuda/bin:${PATH}

# Install Rust (for kz-selfplay)
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y \
    && echo 'source $HOME/.cargo/env' >> /etc/bash.bashrc
ENV PATH=/root/.cargo/bin:${PATH}

# Workspace
WORKDIR /workspace

# Allow building a specific ref if needed
ARG KZERO_REF=master
RUN git clone --branch ${KZERO_REF} --single-branch https://github.com/mmai/kZero.git

WORKDIR /workspace/kZero

# Install Python deps from repo
RUN pip install --no-cache-dir -r python/requirements.txt

# (Optional) Prebuild the Rust binary at image build time.
# Uncomment if you want the selfplay binary baked into the image.
# RUN cargo build --release --manifest-path rust/Cargo.toml -p kz-selfplay

# Default command
CMD ["/bin/bash"]
