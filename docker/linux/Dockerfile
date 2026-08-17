ARG CROSS_BASE_IMAGE=ghcr.io/cross-rs/x86_64-unknown-linux-gnu:latest
FROM ${CROSS_BASE_IMAGE}

ARG CROSS_DEB_ARCH
RUN dpkg --add-architecture "${CROSS_DEB_ARCH}" \
    && apt-get update \
    && apt-get install --assume-yes --no-install-recommends \
        "libasound2-dev:${CROSS_DEB_ARCH}" \
        "libwayland-dev:${CROSS_DEB_ARCH}" \
    && rm -rf /var/lib/apt/lists/*

ENV PKG_CONFIG_ALLOW_CROSS=1
