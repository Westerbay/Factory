FROM debian:bookworm-slim@sha256:99c0b25c46d0214d5a2091092e7908907dcaecd6a962cde0ac700acee222b6f3 AS packages
RUN printf 'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20261027T000000Z bookworm main\ndeb [check-valid-until=no] http://snapshot.debian.org/archive/debian-security/20261027T000000Z bookworm-security main\n' > /etc/apt/sources.list \
    && rm -f /etc/apt/sources.list.d/debian.sources

FROM packages AS build
RUN set -eu; \
    apt-get update; \
    apt-get install -y --no-install-recommends ca-certificates curl xz-utils build-essential libltdl-dev libxml2-dev libexpat1-dev; \
    fetch_squid() { \
      curl --fail --silent --show-error --location --retry 2 --retry-all-errors --connect-timeout 10 --max-time 30 \
        --proto '=https' --proto-redir '=https' --tlsv1.2 "$1" -o /tmp/squid.tar.xz; \
    }; \
    fetch_squid https://deb.debian.org/debian/pool/main/s/squid/squid_7.7.orig.tar.xz \
      || fetch_squid https://github.com/squid-cache/squid/releases/download/SQUID_7_7/squid-7.7.tar.xz; \
    echo 'e3bd613b91b1c498ec2992276063342a85cd6edddd5521294e04f44bc055da9b  /tmp/squid.tar.xz' | sha256sum --check --strict; \
    tar -xJf /tmp/squid.tar.xz -C /tmp
WORKDIR /tmp/squid-7.7
RUN ./configure --prefix=/opt/squid --with-default-user=proxy --disable-auth --disable-snmp --disable-icmp --disable-ident-lookups --disable-esi --disable-wccp --disable-wccpv2 --disable-htcp --disable-cache-digests --disable-delay-pools --without-openssl \
    && make -j2 && make install \
    && cp COPYING CONTRIBUTORS /opt/squid/

FROM packages AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends nftables util-linux libltdl7 libxml2 libexpat1 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 factory-proxy && useradd --uid 10001 --gid 10001 --no-create-home factory-proxy
COPY --from=build /opt/squid /opt/squid
COPY squid.conf /etc/factory/squid.conf
COPY entrypoint.sh /usr/local/bin/factory-egress
RUN chmod 0555 /usr/local/bin/factory-egress
LABEL io.factory.egress.image="1" io.factory.egress.squid="7.7"
USER 0:0
ENTRYPOINT ["/usr/local/bin/factory-egress"]

# Selected only by the explicit integration command; never prepared for production.
FROM runtime AS fixture
RUN apt-get update && apt-get install -y --no-install-recommends python3 \
    && rm -rf /var/lib/apt/lists/* \
    && printf '\npositive_dns_ttl 1 seconds\nnegative_dns_ttl 1 seconds\n' >> /etc/factory/squid.conf
COPY fixture.py /opt/factory-fixture.py

FROM runtime AS final
