# Container egress

Development builds the pinned Squid 7.7 image with `scripts/egress/prepare.mjs`.
Production uses the image digest recorded in the release bundle.
The Rust supervisor owns creation and inspection of the shared outbound bridge,
proxy and per-environment internal bridges. Existing user containers are never
attached or replaced by this module. All addresses in the operator host interface
inventory are denied. Production omits RFC 1918 interface addresses from that
inventory because both network rules already reject those ranges. Preparation
refuses to publish a changed image or interface inventory while an incompatible proxy exists; the previous `egress.json` and
`prepared.json` remain intact. Preparation never removes a proxy or user container.

Production release upgrades verify the previous proxy and replace it when the
image or host interface inventory changes. The updater stops and restores
workspaces according to their saved intent. See [production deployment](../../docs/DEPLOYMENT.md).

## Manual proxy maintenance for development

After changing host interfaces or the proxy image, use this maintenance sequence.
The old inventory does not cover newly added host addresses, so stop environments
before changing interfaces and keep them stopped until preparation succeeds.

1. Stop all user environments through Factory and wait for their stopped states.
   Stop the API so its exclusive supervisor cannot restart or create resources.
2. Read the installation UUID and old proxy image ID from the local prepared
   `installation.json` and `egress.json`. Inspect the exact
   `factory-egress-<installation UUID>` container with `docker container inspect`.
   Record its full 64-character ID. Verify its exact name, image ID and labels:
   `io.factory.installation` equals that UUID, `io.factory.egress.policy=1`,
   `io.factory.egress.role=shared`, `io.factory.egress.image=1`, and
   `io.factory.egress.squid=7.7`. Verify that `Mounts` is empty. If any identity
   or ownership field differs, stop and investigate; do not remove that container.
3. The operator may then run `docker container stop <verified full proxy ID>`,
   inspect that same ID again to verify it is stopped and still matches the
   previous checks, and run `docker container rm <verified full proxy ID>`.
   Do not use force, volume-removal flags, name-based mass removal or prune.
   Keep every user container, home volume and network.
4. Repeat `pnpm environments:prepare`, restart the API, then Start the user
   environments. The supervisor recreates the proxy and reconnects each existing
   owned user network when starting its environment. Homes remain unchanged.
   If preparation fails, leave environments stopped and correct the reported
   failure before retrying. A missing proxy alone never authorizes user removal.

User bridges use `gateway_mode_ipv4=isolated` and disable IPv6. The proxy has no
published ports, mounts, Docker socket or user credentials. Its namespace has
forwarding disabled both through sysctls and nftables. Only its HTTP port 3128
accepts incoming new connections. Squid accepts CONNECT to OpenAI, Anthropic, GitHub and the npm registry
names listed in [squid.conf](squid.conf) on port 443, denies special/private addresses
and does not intercept TLS or log tunnels. Every process in an environment can reach the
permitted SSH endpoint; full-access agents can use that member’s own GitHub SSH key.

The entrypoint requires init-only NET_ADMIN, SETUID, SETGID, SETPCAP and CHOWN.
After installing the firewall, `setpriv` irreversibly clears the bounding,
inheritable and ambient capability sets, switches to UID/GID 10001, and applies
no-new-privileges. Squid itself is PID 1; there is no privileged init process left.
The immutable root filesystem prevents the proxy from editing its policy.

DNS queries from Squid can reach only Docker's embedded resolver at 127.0.0.11.
Docker DNATs that resolver's port before the filter hooks, so rules use the
original conntrack destination port on loopback input/output. Docker's upstream
DNS forwarding sockets are owned by UID 0 inside the namespace. A separate rule
allows only those root-owned UDP/TCP port-53 packets; Squid UID 10001 cannot use
it. This is infrastructure DNS access, not an HTTP CONNECT or private-network
exception for the user or proxy process.

For TCP connections made by Squid, the kernel rejects special/private and
operator host IPv4 addresses regardless of DNS cache changes. IPv6 is disabled
and denied. A mixed DNS answer is rejected by Squid's `dst` ACL; the firewall is
the independent protection on the actual connected address.

Run `node scripts/egress/integration.mjs` explicitly to build disposable images
and exercise Docker. The DNS fixture target is excluded from the final runtime
image. It simulates a public address on an internal isolated bridge and a private
target, checks a working tunnel, mixed answers, rebinding, direct connection
denial and the root-only resolver exception. Other fixtures check per-user
isolation against a listening service with a positive client on the target bridge,
no default route, dropped capabilities and local shell availability
with the proxy stopped. Every resource is removed only after matching the test
installation label; no user volume is involved and no prune is used.
