FROM debian:bookworm-slim@sha256:99c0b25c46d0214d5a2091092e7908907dcaecd6a962cde0ac700acee222b6f3 AS codex
# Immutable Debian snapshot and official release asset checksums make rebuilds explicit.
RUN printf '%s\n' 'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20261027T000000Z bookworm main' > /etc/apt/sources.list \
    && rm /etc/apt/sources.list.d/debian.sources \
    && apt-get update && apt-get install -y --no-install-recommends ca-certificates curl jq
RUN set -eu; \
    case "$(dpkg --print-architecture)" in \
      amd64) target=x86_64-unknown-linux-musl; checksum=4fcc47ab57f52ff75363951a8761146cd10c8288bd86fed45487dbb204a16b71 ;; \
      arm64) target=aarch64-unknown-linux-musl; checksum=7f0fe42ff22ecfa3a47bc4a34f5b22c4218b431a4ec0aba51c7d98299f07900c ;; \
      *) echo 'Unsupported Codex architecture' >&2; exit 1 ;; \
    esac; \
    curl --fail --location --proto '=https' --proto-redir '=https' --tlsv1.2 \
      "https://github.com/openai/codex/releases/download/rust-v0.160.0/codex-package-${target}.tar.gz" -o /tmp/codex.tar.gz; \
    echo "$checksum  /tmp/codex.tar.gz" | sha256sum --check --strict; \
    mkdir /opt/codex; tar -xzf /tmp/codex.tar.gz -C /opt/codex; \
    test "$(/opt/codex/bin/codex --version)" = 'codex-cli 0.160.0'; \
    mkdir /tmp/schema-home; \
    CODEX_HOME=/tmp/schema-home /opt/codex/bin/codex app-server generate-json-schema --out /tmp/schema; \
    jq -e '.oneOf[] | select(.properties.type.enum == ["chatgptDeviceCode"])' /tmp/schema/v2/LoginAccountParams.json >/dev/null; \
    jq -e '.oneOf[] | select(.properties.type.enum == ["chatgptDeviceCode"]) | .required | sort == ["loginId", "type", "userCode", "verificationUrl"]' /tmp/schema/v2/LoginAccountResponse.json >/dev/null; \
    test -s /tmp/schema/v2/GetAccountResponse.json; \
    test -s /tmp/schema/v2/CancelLoginAccountParams.json; \
    test -s /tmp/schema/v2/LogoutAccountResponse.json

FROM debian:bookworm-slim@sha256:99c0b25c46d0214d5a2091092e7908907dcaecd6a962cde0ac700acee222b6f3
LABEL io.factory.environment.image="3"
LABEL io.factory.codex.version="0.160.0"
RUN printf '%s\n' 'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20261027T000000Z bookworm main' \
      'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian-security/20261027T000000Z bookworm-security main' > /etc/apt/sources.list \
    && rm /etc/apt/sources.list.d/debian.sources \
    && apt-get update && apt-get install -y --no-install-recommends bash tmux ca-certificates util-linux \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd -g 10000 factory \
    && useradd -u 10000 -g factory -d /home/factory -s /bin/bash factory \
    && mkdir -p /home/factory/Documents \
    && chown -R 10000:10000 /home/factory
COPY --from=codex /opt/codex /opt/codex
COPY tmux.conf /etc/factory/tmux.conf
COPY auth-controller.sh /usr/local/bin/factory-auth-controller
RUN chmod 0555 /usr/local/bin/factory-auth-controller \
    && ln -s /opt/codex/bin/codex /usr/local/bin/codex
ENV HOME=/home/factory
USER 10000:10000
WORKDIR /home/factory/Documents
ENTRYPOINT ["/bin/sleep"]
CMD ["infinity"]
