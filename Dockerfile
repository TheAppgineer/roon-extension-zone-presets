ARG build_arch=amd64

# Phase 1: Create the extension binary
FROM multiarch/alpine:${build_arch}-v3.14 AS target-build

ARG build_arch

RUN addgroup -g 1000 worker && \
    adduser -u 1000 -G worker -s /bin/sh -D worker && \
    apk add --no-cache curl gcc musl-dev

WORKDIR /home/worker

COPY Cargo.toml Cargo.lock LICENSE README.md /home/worker/
COPY src /home/worker/src/

RUN chown -R worker:worker /home/worker

USER worker

RUN curl --proto '=https' --tlsv1.2 -o rustup-init.sh -sSf https://sh.rustup.rs && \
    sh ./rustup-init.sh -y --no-modify-path && \
    PATH=$PATH:/home/worker/.cargo/bin cargo build --jobs $(grep -c ^processor /proc/cpuinfo) --release


# Phase 2: Create the run-time image containing the extension binary
FROM multiarch/alpine:${build_arch}-v3.14

RUN addgroup -g 1000 worker && \
    adduser -u 1000 -G worker -s /bin/sh -D worker

WORKDIR /home/worker

USER worker

COPY --from=target-build /home/worker/target/release/roon-extension-zone-presets /home/worker/

CMD [ "./roon-extension-zone-presets" ]
