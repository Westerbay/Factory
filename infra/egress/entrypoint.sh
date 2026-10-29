#!/bin/sh
set -eu
umask 077
# Docker sets these before init. Never repair a weaker namespace silently.
test "$(cat /proc/sys/net/ipv4/ip_forward)" = 0
test "$(cat /proc/sys/net/ipv6/conf/all/forwarding)" = 0
test "$(cat /proc/sys/net/ipv6/conf/all/disable_ipv6)" = 1
mkdir -m 0700 /tmp/factory-egress
# Only literal IPv4 addresses supplied by trusted operator preparation enter rules.
test -n "${FACTORY_HOST_IPV4:-}"
test "${#FACTORY_HOST_IPV4}" -le 8192
printf '%s\n' "$FACTORY_HOST_IPV4" | grep -Eq '^([0-9]{1,3}\.){3}[0-9]{1,3}(,([0-9]{1,3}\.){3}[0-9]{1,3})*$'
printf 'acl host_addresses dst %s\n' "$(printf '%s' "$FACTORY_HOST_IPV4" | tr ',' ' ')" > /tmp/factory-egress/host-acl.conf
cat > /tmp/factory-egress/rules.nft <<EOF
table inet factory_egress {
 set blocked_ipv4 {
  type ipv4_addr; flags interval;
  elements = { 0.0.0.0/8, 10.0.0.0/8, 100.64.0.0/10, 127.0.0.0/8, 169.254.0.0/16, 172.16.0.0/12, 192.0.0.0/24, 192.0.2.0/24, 192.88.99.0/24, 192.168.0.0/16, 198.18.0.0/15, 198.51.100.0/24, 203.0.113.0/24, 224.0.0.0/4, 240.0.0.0/4 }
 }
 set host_ipv4 { type ipv4_addr; elements = { $FACTORY_HOST_IPV4 } }
 chain forward { type filter hook forward priority 0; policy drop; }
 chain input {
  type filter hook input priority 0; policy drop;
  ct state established,related accept
  iifname "lo" ip daddr 127.0.0.11 meta l4proto udp ct original proto-dst 53 accept
  iifname "lo" ip daddr 127.0.0.11 meta l4proto tcp ct original proto-dst 53 accept
  meta nfproto ipv4 tcp dport 3128 accept
 }
 chain output {
  type filter hook output priority 0; policy drop;
  ct state established,related accept
  ip daddr 127.0.0.11 meta l4proto udp ct original proto-dst 53 accept
  ip daddr 127.0.0.11 meta l4proto tcp ct original proto-dst 53 accept
  # Docker's embedded resolver forwards using root-owned sockets in this namespace.
  # Squid is UID10001 with no capabilities and cannot use this infrastructure exception.
  meta skuid 0 udp dport 53 counter accept
  meta skuid 0 tcp dport 53 counter accept
  ip daddr @host_ipv4 reject
  ip daddr @blocked_ipv4 reject
  meta nfproto ipv4 tcp dport 443 accept
 }
}
EOF
/usr/sbin/nft --check --file /tmp/factory-egress/rules.nft
/usr/sbin/nft --file /tmp/factory-egress/rules.nft
chown -R 10001:10001 /tmp/factory-egress
# NET_ADMIN is init-only; Squid cannot change the firewall or regain root/capabilities.
exec /usr/bin/setpriv --reuid=10001 --regid=10001 --clear-groups --bounding-set=-all --inh-caps=-all --ambient-caps=-all --no-new-privs \
 /opt/squid/sbin/squid -N -f /etc/factory/squid.conf
