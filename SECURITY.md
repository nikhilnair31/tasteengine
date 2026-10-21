# Security policy

Taste Engine stores a personal library and the credentials that unlock it. Treat
a deployment as private by default.

## Reporting a vulnerability

Open a private security advisory on GitHub, or email the maintainer listed in
the repository profile. Do not open a public issue for a vulnerability.

Include: the version or commit, what you expected, what happened, and the
smallest reproduction you have. Please give a maintainer a reasonable window to
fix it before you publish anything.

## Supported versions

Fixes land on `main` and in the latest tagged release. Older tags are not
patched. If you run a fork, you own the backport.

## Deploying safely

The defaults are for a single user on a trusted network. Before you expose an
instance to the internet:

- Set a real `TV_USER` and a long `TV_PASS`. The `change-me` values in
  `.env.example` are placeholders, and HTTP Basic over plain HTTP sends them in
  the clear.
- Terminate TLS in front of the container. Caddy, nginx or any reverse proxy
  with a certificate works. The app speaks HTTP only.
- Bind the published port to a private interface, or put the proxy on the same
  Docker network, instead of publishing `0.0.0.0:8770`.
- Keep `data/` out of anything you publish. It holds `vault.db`, every saved
  image and video, and the API tokens you created, which are stored as SHA-256
  hashes.

## What the software does with your credentials

- **Basic auth** is compared in constant time against `TV_USER` and `TV_PASS`.
- **API tokens** are stored as SHA-256 hashes, never plaintext. The plaintext is
  shown once, when you create it. `DELETE /api/tokens/{id}` revokes it.
- **Source cookies are never stored.** Capture runs inside your own browser tab.
  The server never receives a session cookie for X, Instagram or Pinterest, and
  there is no scraping server anywhere in this project.
- **The OpenRouter key** is read from `.env` and used only for outbound calls to
  the model you configured. It is never logged or returned by an endpoint.
- **`share` tokens** (`TV_SHARE_TOKEN`) bypass auth for specific routes. Leave
  it unset unless you are using the share path, and rotate it if it leaks.

## Out of scope

- Rate limits or account consequences on third-party platforms. The per-source
  gate paces requests, but the account and its terms are yours.
- Data loss from deleting `data/` or running a bulk delete without a backup.
  `make backup` exists for that reason.
- Anything reachable only because you published port 8770 directly to the
  internet without TLS.
