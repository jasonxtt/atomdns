# AtomDNS container image. The build context is the staging directory made by
# scripts/build-docker-image.sh (release artifacts, nothing is compiled here):
#   bin/{amd64,arm64,armv7}/atomdns   glibc programs (static musl ARM ones for releases before the -gnu packages)
#   assets/                           config_rust.zip, config_rust_up.zip, atomdns-webui.tar.gz
# There is no RUN step for the target platform, so a multi-arch build needs no emulation.

# CA roots and zoneinfo are architecture independent: they are installed on the build platform and copied.
FROM --platform=$BUILDPLATFORM debian:bookworm-slim AS data
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends ca-certificates tzdata \
    && rm -rf /var/lib/apt/lists/*

FROM debian:bookworm-slim

ARG TARGETARCH
ARG TARGETVARIANT
ARG VERSION=dev
ARG VCS_REF=unknown

LABEL org.opencontainers.image.title="atomdns" \
      org.opencontainers.image.description="AtomDNS: a Rust DNS splitter with WebUI (MosDNS-compatible)" \
      org.opencontainers.image.source="https://github.com/jasonxtt/atomdns" \
      org.opencontainers.image.licenses="GPL-3.0-only" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${VCS_REF}"

COPY --from=data /etc/ssl/certs /etc/ssl/certs
COPY --from=data /usr/share/ca-certificates /usr/share/ca-certificates
COPY --from=data /usr/share/zoneinfo /usr/share/zoneinfo
# armv7 is TARGETARCH=arm + TARGETVARIANT=v7; amd64 and arm64 have no variant.
COPY --chmod=0755 bin/${TARGETARCH}${TARGETVARIANT}/atomdns /usr/local/bin/atomdns
COPY assets/ /usr/share/atomdns/

# Container mode: the in-app updater is off (upgrade by pulling a new image) and the start-up
# step fills an empty /cus/atomdns from the image and brings an older one up to date.
ENV ATOMDNS_CONTAINER=1
WORKDIR /cus/atomdns
VOLUME ["/cus/atomdns"]

# DNS and the management API / WebUI. A host-network container ignores EXPOSE.
EXPOSE 53/udp 53/tcp 9099/tcp

ENTRYPOINT ["/usr/local/bin/atomdns"]
CMD ["start", "-d", "/cus/atomdns", "-c", "config_custom.yaml"]
