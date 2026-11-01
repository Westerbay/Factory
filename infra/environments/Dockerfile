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

# Official, unmodified native Claude Code release. Pin both architectures independently.
RUN set -eu; \
    case "$(dpkg --print-architecture)" in \
      amd64) platform=linux-x64; checksum=5cd90aabd83f8a15136c35aa37bb1d92b348993573316643dc3fe4e04afbf88f ;; \
      arm64) platform=linux-arm64; checksum=3dd0f96d7ada463152d20300186f6cfc6ab94b57e218f49e3ac86db42ac695a6 ;; \
      *) echo 'Unsupported Claude Code architecture' >&2; exit 1 ;; \
    esac; \
    mkdir /opt/claude; \
    curl --fail --location --proto '=https' --proto-redir '=https' --tlsv1.2 \
      "https://downloads.claude.ai/claude-code-releases/2.1.284/${platform}/claude" -o /opt/claude/claude; \
    echo "$checksum  /opt/claude/claude" | sha256sum --check --strict; \
    chmod 0555 /opt/claude/claude; \
    /opt/claude/claude --version | grep -Fx '2.1.284 (Claude Code)'

# Verify the pinned CLI's initialization and model catalog with an empty home.
RUN set -eu; \
    mkdir -p /tmp/claude-probe; \
    printf '%s\n' '{"type":"control_request","request_id":"factory-probe","request":{"subtype":"initialize","hooks":{},"mcpServers":{}}}' \
      | HOME=/tmp/claude-probe CLAUDE_CONFIG_DIR=/tmp/claude-probe/config \
        /opt/claude/claude --print --input-format stream-json --output-format stream-json --verbose \
      | jq -e 'select(.type == "control_response" and .response.request_id == "factory-probe" and .response.subtype == "success") | .response.response | (.models | type == "array" and length > 0) and (.account.tokenSource == "none")' >/dev/null

# Pin and verify the upstream Oh My Zsh source independently of the mutable branch.
RUN curl --fail --location --proto '=https' --proto-redir '=https' --tlsv1.2 \
      https://codeload.github.com/ohmyzsh/ohmyzsh/tar.gz/17a2b4c3a5855524ad4772a17ada0dc474e2fdf3 -o /tmp/oh-my-zsh.tar.gz \
    && echo 'eb6f2788316ff75f9be60c00372da0da97d4bcab202d64a9229e1f6365166932  /tmp/oh-my-zsh.tar.gz' | sha256sum --check --strict \
    && mkdir /opt/oh-my-zsh \
    && tar -xzf /tmp/oh-my-zsh.tar.gz --strip-components=1 -C /opt/oh-my-zsh \
    && chmod -R go-w /opt/oh-my-zsh

FROM debian:bookworm-slim@sha256:99c0b25c46d0214d5a2091092e7908907dcaecd6a962cde0ac700acee222b6f3
LABEL io.factory.environment.image="5"
LABEL io.factory.codex.version="0.160.0"
LABEL io.factory.claude.version="2.1.284"
RUN printf '%s\n' 'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20261027T000000Z bookworm main' \
      'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian-security/20261027T000000Z bookworm-security main' > /etc/apt/sources.list \
    && rm /etc/apt/sources.list.d/debian.sources \
    && apt-get update && apt-get install -y --no-install-recommends bash zsh tmux git neofetch ca-certificates util-linux openssh-client netcat-openbsd python3 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd -g 10000 factory \
    && useradd -u 10000 -g factory -d /home/factory -s /bin/zsh factory \
    && mkdir -p /home/factory/Documents \
    && chown -R 10000:10000 /home/factory
COPY --from=codex /opt/codex /opt/codex
COPY --from=codex /opt/claude /opt/claude
COPY --from=codex /opt/oh-my-zsh /opt/oh-my-zsh
COPY tmux.conf /etc/factory/tmux.conf
COPY zshrc /etc/factory/zshrc
COPY auth-controller.sh /usr/local/bin/factory-auth-controller
COPY project-controller.py /usr/local/bin/factory-project-controller
COPY github-ssh-config /etc/factory/github-ssh-config
COPY github-known-hosts /etc/factory/github-known-hosts
# Avoid an interactive first-run wizard when the persistent home has no zsh files.
RUN chmod 0555 /usr/local/bin/factory-auth-controller \
    && chmod 0555 /usr/local/bin/factory-project-controller \
    && mkdir -p /etc/ssh/ssh_config.d \
    && cp /etc/factory/github-ssh-config /etc/ssh/ssh_config.d/10-factory-github.conf \
    && git config --system include.path /home/factory/.factory/git-identity \
    && printf '\nsource /etc/factory/zshrc\n' >> /etc/zsh/zshrc \
    && : > "/usr/share/zsh/$(zsh -fc 'print -r -- $ZSH_VERSION')/scripts/newuser" \
    && ln -s /opt/codex/bin/codex /usr/local/bin/codex \
    && ln -s /opt/claude/claude /usr/local/bin/claude
ENV HOME=/home/factory
USER 10000:10000
WORKDIR /home/factory/Documents
ENTRYPOINT ["/usr/local/bin/factory-project-controller"]
CMD ["--bootstrap"]
