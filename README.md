[中文文档](docs/README.zh-CN.md) | English

<h1 align="center">ipaTool</h1>

<p align="center">
  Mobile-first IPA download, archive & installation manager
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Vue-3-4FC08D?logo=vue.js" alt="Vue 3">
  <img src="https://img.shields.io/badge/Rust-000000?logo=rust" alt="Rust">
  <img src="https://img.shields.io/badge/License-MIT-blue" alt="MIT">
  <a href="https://github.com/ruanrrn/ipaTool/releases"><img src="https://img.shields.io/github/v/release/ruanrrn/ipaTool?label=release" alt="GitHub release"></a>
</p>

---

## Features

- Download delisted apps
- Purchase free apps
- Complete version history
- Version favorites & notes
- OTA install or IPA export
- Multi Apple ID management

## Preview

| Home | Queue | Favorites |
|:----:|:-----:|:---------:|
| <img src="docs/screenshots/home-search.jpg" width="220"> | <img src="docs/screenshots/queue-active.jpg" width="220"> | <img src="docs/screenshots/archive-favorites.jpg" width="220"> |

| Settings | Add Account | Version Select |
|:--------:|:-----------:|:--------------:|
| <img src="docs/screenshots/settings-appearance.jpg" width="220"> | <img src="docs/screenshots/add-apple-id.jpg" width="220"> | <img src="docs/screenshots/app-detail.jpg" width="220"> |

## Quick Start

### Method 1: One-Click Install (Recommended)

Run the command below — it launches an interactive management panel:

```bash
bash <(curl -fsSL https://cdn.jsdelivr.net/gh/ruanrrn/ipaTool@main/scripts/install.sh)
```

> If jsDelivr is slow in your region, use the GitHub raw link instead:
> ```bash
> bash <(curl -fsSL https://raw.githubusercontent.com/ruanrrn/ipaTool/main/scripts/install.sh)
> ```

After installation, reopen the panel anytime:

```bash
sudo bash /opt/ipatool/manager.sh
```

---

### Method 2: Docker

**Using docker-compose:**

Create a `docker-compose.yml`:

```yaml
version: '3.8'

services:
  ipatool:
    image: heard/ipatool
    container_name: ipatool
    ports:
      - "8080:8080"
    volumes:
      - ipa-data:/app/data
      - ipa-downloads:/app/downloads
    restart: unless-stopped

volumes:
  ipa-data:
  ipa-downloads:
```

Then start:

```bash
docker-compose up -d   # → http://localhost:8080
```

**Using docker run:**

```bash
docker run -d -p 8080:8080 --name ipatool heard/ipatool:latest
```

**View / Reset Admin Password:**

Set an initial password at startup (recommended):

```bash
docker run -d -p 8080:8080 \
  -e IPA_ADMIN_INITIAL_PASSWORD='your-strong-password' \
  --name ipatool heard/ipatool:latest
```

If not set, retrieve the auto-generated password from logs:

```bash
docker logs ipatool 2>&1 | grep 'Generated one-time admin password'
```

To reset the password:

```bash
docker exec -i ipatool ./server reset-admin-password --username admin --password-stdin <<< 'new-password'
```

---

### Method 3: Run from Source

**Prerequisites:** Node.js 18+ · pnpm · Rust 1.85+

**Development:**

```bash
pnpm install
pnpm run dev                        # Frontend → localhost:5173
cd server && cargo run --bin server # Backend → localhost:8080
```

**Production build:**

```bash
pnpm run build
rm -rf server/dist && cp -a dist/. server/dist/
cd server && cargo run --bin server
```

**View / Reset Admin Password:**

Set an initial password via environment variable (recommended):

```bash
cd server
IPA_ADMIN_INITIAL_PASSWORD='your-secure-password' cargo run --bin server
```

If not set, check the terminal output for:

```text
[SECURITY] Generated one-time admin password for first run: ...
```

To reset the password:

```bash
cd server
printf '%s' 'new-password' | cargo run --bin server -- reset-admin-password --username admin --password-stdin
```

If the database is in a custom location:

```bash
DATABASE_PATH=/path/to/ipa-webtool.db cargo run --bin server -- reset-admin-password --username admin --password-stdin
```

---

## HTTPS Setup (Required for OTA)

OTA installation requires HTTPS. Common approaches:

| Solution | Description |
|----------|-------------|
| **Nginx + Let's Encrypt** | Use Certbot for free certificates, Nginx reverse proxy to port 8080 |
| **Cloudflare Tunnel** | Expose via Cloudflare tunnel, no public port required |

Example Nginx reverse proxy config:

```nginx
server {
    listen 443 ssl;
    server_name your-domain.com;
    ssl_certificate     /etc/letsencrypt/live/your-domain.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/your-domain.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

`X-Forwarded-Proto` lets the backend generate `https://` OTA links and mark the login cookie `Secure`.

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `PORT` | `8080` | Listen port (binds `0.0.0.0:$PORT`) |
| `BIND_ADDRESS` | — | Full listen address, e.g. `127.0.0.1:8080`; overrides `PORT` |
| `IPA_TOOL_PUBLIC_BASE_URL` | — | Public origin used in OTA/download links, e.g. `https://ipa.example.com` |
| `FORCE_SECURE_COOKIE` | auto | `1` always sets the `Secure` cookie flag, `0` never; by default it follows the request scheme |
| `TRUSTED_PROXIES` | loopback + private ranges | Peers whose `X-Forwarded-For` / `X-Real-IP` is trusted for login rate limiting: comma-separated IPs/CIDRs, `*` for any, `none` to ignore the headers |
| `IPA_ADMIN_INITIAL_PASSWORD` | random | Admin password used on first start only |
| `DATABASE_PATH` | `data/ipa-webtool.db` | SQLite database location |

## Tech Stack

| Frontend | Backend |
|----------|---------|
| Vue 3 + Vite 6 | Rust |
| Pinia | Actix Web |
| Tailwind CSS | rusqlite |
| | reqwest · tokio · OpenSSL (vendored) |

## Project Structure

```
├── frontend/            # Vue 3 frontend
│   ├── index.html       # Vite entry
│   ├── vite.config.js
│   ├── tailwind.config.js
│   ├── postcss.config.js
│   ├── eslint.config.js
│   ├── .prettierrc
│   ├── main.js          # App entry
│   ├── App.vue          # Root component
│   ├── components/      # Components
│   ├── composables/     # Composables
│   ├── stores/          # Pinia stores
│   └── utils/           # Utilities
├── server/              # Rust backend (Actix Web)
│   ├── src/
│   ├── Cargo.toml
│   └── rustfmt.toml
├── scripts/             # Build & ops scripts
│   ├── install.sh       # One-click management script
│   ├── verify-build.sh
│   ├── sync-version.sh
│   └── check-no-hardcoded-colors.sh
├── docs/                # Documentation & screenshots
│   ├── screenshots/
│   ├── learnings/
│   └── README.zh-CN.md  # Chinese documentation
├── .github/workflows/   # CI/CD (ci, docker, release)
├── Dockerfile
├── docker-compose.yml
├── .npmrc
├── package.json
├── pnpm-lock.yaml
└── README.md
```

## License

[MIT](LICENSE)
