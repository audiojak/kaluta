# openagc-rules, the rules server for cloud agents (docs/rules-server.md).
#
# Build from the repository root:
#   docker build -f crates/rules-server/Dockerfile -t openagc-rules .
# Run (plain HTTP on 8787; put a TLS proxy in front):
#   docker run -d --name openagc-rules -p 127.0.0.1:8787:8787 -v openagc-rules:/data openagc-rules

FROM rust:1-bookworm AS build
WORKDIR /src
# The toolchain the repository pins (rust-toolchain.toml), without the
# macOS target it also lists.
ENV RUSTUP_TOOLCHAIN=1.98.1
RUN rustup toolchain install 1.98.1 --profile minimal
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY xtask xtask
RUN cargo build --release --locked -p rules-server --bin openagc-rules \
 && mkdir -p /out/data \
 && cp target/release/openagc-rules /out/openagc-rules

# glibc and libgcc only: SQLite is compiled into the binary.
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /out/openagc-rules /openagc-rules
# The volume's mount point, owned by the image's non-root user (65532) so a
# new named volume is writable by it.
COPY --from=build --chown=65532:65532 /out/data /data
ENV OPENAGC_RULES_LISTEN=0.0.0.0:8787 \
    OPENAGC_RULES_DATA_DIR=/data \
    OPENAGC_RULES_LOG=info
USER 65532:65532
VOLUME ["/data"]
EXPOSE 8787
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
  CMD ["/openagc-rules", "healthcheck"]
ENTRYPOINT ["/openagc-rules"]
CMD ["serve"]
