FROM debian:bookworm-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc AS codex
# Immutable Debian snapshot and official artifact checksums make rebuilds explicit.
RUN printf '%s\n' 'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20261027T000000Z bookworm main' > /etc/apt/sources.list \
    && rm /etc/apt/sources.list.d/debian.sources \
    && apt-get -o Acquire::Retries=3 -o APT::Update::Error-Mode=any update && apt-get -o Acquire::Retries=3 install -y --no-install-recommends ca-certificates curl jq
# Use the official npm platform bundle; only its native runtime is needed.
RUN set -eu; \
    case "$(dpkg --print-architecture)" in \
      amd64) platform=linux-x64; target=x86_64-unknown-linux-musl; checksum=288ffbdcea86ac7991d7cb3bc9aec4d4da95eab4f4cb7971af4b3c4b5a91da6eb353a41117f1e5479dbd8c02cb879be37549ec453e00868c6dd1ac5be2463df6 ;; \
      arm64) platform=linux-arm64; target=aarch64-unknown-linux-musl; checksum=56755db12d3a6250ec2fc3b06898d0df176a749346e22f9e049055dcbcad75d927cd2685feeae28cbcd5837a6d930be88fc0bb1a7ee25a9544fb2f1611a3e20a ;; \
      *) echo 'Unsupported Codex architecture' >&2; exit 1 ;; \
    esac; \
    curl --fail --silent --show-error --location --retry 3 --retry-all-errors --connect-timeout 15 --retry-max-time 180 --max-time 180 --proto '=https' --proto-redir '=https' --tlsv1.2 \
      "https://registry.npmjs.org/@openai/codex/-/codex-0.160.0-${platform}.tgz" -o /tmp/codex.tgz; \
    echo "$checksum  /tmp/codex.tgz" | sha512sum --check --strict; \
    mkdir /tmp/codex-package; tar -xzf /tmp/codex.tgz -C /tmp/codex-package; \
    mv "/tmp/codex-package/package/vendor/$target" /opt/codex; \
    rm -r /tmp/codex.tgz /tmp/codex-package; \
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
    curl --fail --silent --show-error --location --retry 3 --retry-all-errors --connect-timeout 15 --retry-max-time 180 --max-time 180 --proto '=https' --proto-redir '=https' --tlsv1.2 \
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
RUN curl --fail --silent --show-error --location --retry 3 --retry-all-errors --connect-timeout 15 --retry-max-time 180 --max-time 180 --proto '=https' --proto-redir '=https' --tlsv1.2 \
      https://codeload.github.com/ohmyzsh/ohmyzsh/tar.gz/17a2b4c3a5855524ad4772a17ada0dc474e2fdf3 -o /tmp/oh-my-zsh.tar.gz \
    && echo 'eb6f2788316ff75f9be60c00372da0da97d4bcab202d64a9229e1f6365166932  /tmp/oh-my-zsh.tar.gz' | sha256sum --check --strict \
    && mkdir /opt/oh-my-zsh \
    && tar -xzf /tmp/oh-my-zsh.tar.gz --strip-components=1 -C /opt/oh-my-zsh \
    && chmod -R go-w /opt/oh-my-zsh

FROM debian:bookworm-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc
LABEL io.factory.environment.image="5"
LABEL io.factory.codex.version="0.160.0"
LABEL io.factory.claude.version="2.1.284"
RUN printf '%s\n' 'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20261027T000000Z bookworm main' \
      'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian-security/20261027T000000Z bookworm-security main' > /etc/apt/sources.list \
    && rm /etc/apt/sources.list.d/debian.sources \
    && apt-get -o Acquire::Retries=3 -o APT::Update::Error-Mode=any update && apt-get -o Acquire::Retries=3 install -y --no-install-recommends bash zsh tmux git neofetch ca-certificates util-linux openssh-client netcat-openbsd python3 \
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
