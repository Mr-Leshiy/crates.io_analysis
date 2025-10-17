FROM python:3.12-slim-bookworm
COPY --from=docker.io/astral/uv:latest /uv /uvx /bin/

# Install necessary tools for rustup
RUN apt-get update && \
    apt-get install -y curl build-essential && \
    apt-get clean
# Install Rust using rustup
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
# Ensure Cargo's bin directory is in the PATH for subsequent commands
ENV PATH="/root/.cargo/bin:${PATH}"
# install cargo-deny
RUN cargo install --locked --version 0.18.5 cargo-deny

WORKDIR /app
COPY pyproject.toml uv.lock deny.toml gather.py cargo_deny.py .
RUN uv sync
ENTRYPOINT ["uv", "run", "gather.py"]