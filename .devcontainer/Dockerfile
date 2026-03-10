FROM mcr.microsoft.com/devcontainers/javascript-node:20-bookworm

USER root

ENV DEBIAN_FRONTEND=noninteractive
ENV CARGO_HOME=/usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup
ENV PATH=/usr/local/cargo/bin:/usr/local/bin:${PATH}

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        gh \
        git \
        pkg-config \
        python3 \
        python3-pip \
        ripgrep \
    && rm -rf /var/lib/apt/lists/*

RUN curl -LsSf https://astral.sh/uv/install.sh | env UV_INSTALL_DIR=/usr/local/bin sh
RUN curl https://sh.rustup.rs -sSf | sh -s -- -y --profile minimal
RUN npm install -g @anthropic-ai/claude-code @openai/codex

WORKDIR /tmp/tracegrep
COPY Cargo.toml Cargo.lock README.md ./
COPY src ./src
RUN cargo install --path . --locked
RUN rm -rf /tmp/tracegrep

USER node
WORKDIR /workspace
