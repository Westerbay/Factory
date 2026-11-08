# Private production deployment

Factory runs Caddy, the API, PostgreSQL and Docker environments on one dedicated
Linux x86-64 host. The release includes the static web app. Caddy serves it and
proxies `/api` and `/health` to the API at the same public HTTPS origin. Use a
subdomain reserved for Factory, such as `https://factory.example.com`.

Only Caddy publishes ports. PostgreSQL has its own private Docker network. The
API needs the local Docker socket to manage member environments, so access to the
API container or Docker daemon confers host-level privileges. Use a dedicated
host and admit trusted members. Member environments do not receive the socket.
The installation does not cap member CPU, RAM, process count or storage.
Monitor these resources and test real workloads before increasing concurrency.

## Prepare the server

1. Install rootful Docker Engine 28 or later with isolated bridges, the Docker
   Compose plugin, Bash, `flock`, and Node.js 24.22.1. Allow inbound TCP 80 and
   443 for Caddy and restrict SSH to trusted operators. Point
   `factory.example.com` directly at the server in DNS. Caddy obtains and renews
   the certificate. Configure an AAAA record if the server serves IPv6.
2. Register a dedicated GitHub OAuth App. Use `https://factory.example.com` as the
   homepage and `https://factory.example.com/api/auth/github/callback` as the
   callback. Use a separate application for local development.
3. Create `/opt/factory` with mode `0700` and owner root. Copy the
   [server template](../infra/deploy/factory.env.example) to `/opt/factory/.env`,
   make it mode `0600`, and edit it on the server. Set `FACTORY_WEB_ORIGIN` to
   `https://factory.example.com`, using your own subdomain. The API uses the same
   address by default. Set the OAuth values and generate two different passwords
   with `openssl rand -hex 32`. Keep the file outside the release directories.
4. For private images, authenticate the same operator account that will deploy
   using a GitHub classic token with `read:packages` and read access to all
   three GHCR packages. Pass the token through stdin to
   `docker login ghcr.io --username YOUR_GITHUB_USERNAME --password-stdin`.
   Keep this credential on the server.

## Publish and deploy a version

Publish a stable GitHub Release tagged `vX.Y.Z` from a commit on `main`. The
[release workflow](../.github/workflows/publish-release.yml) runs the project
checks, publishes three Linux amd64 images to GHCR and assembles a versioned
server bundle with the static web build. Unchanged workspace and egress sources
reuse their published images. Download the `factory-server` artifact from that workflow run.
It contains a tarball, checksum, and a manifest of exact image digests. Keep a
copy of each deployed bundle outside Actions artifact retention.

Copy the tarball and checksum to the server. From their directory, verify and
extract the bundle into a new version directory. Run these as root or through sudo using the actual release number in place of
`vX.Y.Z`.

```sh
sha256sum --check factory-server.tar.gz.sha256
mkdir -p /opt/factory/releases/vX.Y.Z
tar -xzf factory-server.tar.gz -C /opt/factory/releases/vX.Y.Z
/opt/factory/releases/vX.Y.Z/infra/deploy/server.sh /opt/factory deploy
curl --fail --silent --show-error https://factory.example.com/health/ready
```

Finish or stop active agent work before upgrading. Deployments are serialized.
The command pulls the exact images before stopping the API, then verifies and
records the workspace and egress images and starts Compose. It preserves the
installation state, database and user volumes. PostgreSQL applies the repository
schema during first initialization only and uses a separate non-superuser role
for the API. Future schema changes require an explicit, reviewed upgrade step
before starting an API that expects them. `/health/ready` checks identity and the
database schema. It does not exercise OAuth or the agent runtime.

On the first release, set up the administrator with the explicit CLI and type
the displayed username to confirm:

```sh
/opt/factory/releases/vX.Y.Z/infra/deploy/server.sh /opt/factory compose \
  exec api factory admin setup --provider github --username YOUR_GITHUB_USERNAME
```

Open `https://factory.example.com` and verify GitHub sign-in, workspace startup,
a terminal connection and an agent run with the intended accounts.
Inspect the API and Caddy logs with `server.sh /opt/factory compose logs api caddy`
if a check fails.

## Host the frontend separately

For a separate static host, set both origins explicitly. Use sibling subdomains
under the same domain you control so the browser can send the API's host-only
`Secure`, `HttpOnly`, `SameSite=Lax` session cookie. Only the configured web origin
gets credentialed CORS. The API host serves backend routes in this mode.

```dotenv
FACTORY_WEB_ORIGIN=https://factory.example.com
FACTORY_API_ORIGIN=https://factory-api.example.com
```

Point the API hostname at the server, use the web URL as the OAuth homepage
and the API URL with `/api/auth/github/callback` as the OAuth callback. Existing
installations that only set `FACTORY_DOMAIN` keep their `app.` and `api.` origins.

Build the frontend from the release commit at the repository root using Node.js
24.22.1 and pnpm 12.11.2. Build variables prefixed with `VITE_` are public.

```sh
pnpm install --frozen-lockfile
VITE_API_BASE_URL=https://factory-api.example.com pnpm --filter @workspace/web build
```

Publish `apps/web/dist` on the configured web origin after the matching API is
ready. Configure the static host to serve existing files and fall back to
`index.html` for frontend navigation routes such as `/app`. Revalidate the entry
HTML, `sw.js` and `manifest.webmanifest` on each request. Hashed assets may be
cached immutably. The build includes a [`_headers` example](../apps/web/public/_headers)
for static hosts that support it.

## Recovery

Keep off-host encrypted backups of PostgreSQL, the persistent member home volumes,
and `/opt/factory/state`, which contains the installation identity, preparation
records and repository encryption key. Protect the server configuration and
database credentials separately. Pause writes or coordinate snapshots so the
database and member homes represent the same recovery point. With writes paused,
the release's Compose wrapper can create a PostgreSQL dump:

```sh
server.sh /opt/factory compose exec -T postgres pg_dump -U postgres -Fc factory > database.dump
```

Keep the dump in a private directory before transferring it off the host. Rehearse
restoration on a separate host before relying on this setup. Restore the
installation identity and key, the PostgreSQL roles and data, and the member
volumes with their original names and ownership labels. Retain or repull the
images referenced by those environments. Verify sign-in, project contents and
workspace startup against the restored system. A database dump alone does not
recover unpushed repository changes or stored credentials.

To roll back a compatible API release, run the same deploy command from the
previous bundle. It restores the bundled web build as well. If you host the
frontend separately, restore its matching build on the static host. Verify the
previous API accepts the database schema and the origin settings before rolling
back. Releases without a bundled frontend require `FACTORY_DOMAIN` and separate
frontend hosting. A changed egress image may require the
[explicit proxy maintenance procedure](../infra/egress/README.md). After its
verified removal steps, rerun the production deploy command to prepare the new
proxy. Existing workspace containers keep their recorded image after deployment.
Retain images referenced by those containers.
