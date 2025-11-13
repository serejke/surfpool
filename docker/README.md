# Surfpool Docker Stack

This directory contains the Docker Swarm stack configuration for deploying Surfpool.

## Prerequisites

- Docker Swarm mode enabled (`docker swarm init`)
- Your domain configured and pointing to your server

## Configuration

### 1. Create Environment File

Copy the example environment file and customize it:

```bash
cp .env.example .env
```

Edit `.env` and replace `domain.com` with your actual domain:

```bash
SURFPOOL_DATASOURCE_RPC_URL=https://api.mainnet-beta.solana.com
SURFPOOL_PUBLIC_RPC_URL=https://rpc.solana-surfpool.yourdomain.com
SURFPOOL_PUBLIC_WS_URL=wss://ws.solana-surfpool.yourdomain.com
SURFPOOL_PUBLIC_STUDIO_URL=https://solana-surfpool.yourdomain.com
```

### 2. Update Caddyfile

Edit `Caddyfile` and replace all instances of `domain.com` with your actual domain.

## Deployment

### Initial Deployment

To deploy the stack with environment variables from your `.env` file:

```bash
# Load environment variables and deploy
set -a && source .env && set +a && docker stack deploy -c docker-stack.yaml surfpool
```

Alternatively, you can export variables manually:

```bash
export SURFPOOL_DATASOURCE_RPC_URL=https://api.mainnet-beta.solana.com
export SURFPOOL_PUBLIC_RPC_URL=https://rpc.solana-surfpool.yourdomain.com
export SURFPOOL_PUBLIC_WS_URL=wss://ws.solana-surfpool.yourdomain.com
export SURFPOOL_PUBLIC_STUDIO_URL=https://solana-surfpool.yourdomain.com

docker stack deploy -c docker-stack.yaml surfpool
```

### Updating the Stack

To update the stack with new environment variables or configuration:

```bash
# Load environment variables and redeploy
set -a && source .env && set +a && docker stack deploy -c docker-stack.yaml surfpool
```

Docker Swarm will perform a rolling update, maintaining service availability.

### Restarting Services

To restart a specific service without redeploying:

```bash
docker service update --force surfpool_surfpool
docker service update --force surfpool_caddy
```

To restart the entire stack, redeploy as shown above.

## Updating Caddyfile

If you update the Caddyfile, you need to create a new config version:

1. Update `Caddyfile`
2. Edit `docker-stack.yaml` and increment the config version (e.g., `surfpool_caddyfile_v3` → `surfpool_caddyfile_v4`)
3. Redeploy: `set -a && source .env && set +a && docker stack deploy -c docker-stack.yaml surfpool`

## Management Commands

```bash
# View stack services
docker stack services surfpool

# View service logs
docker service logs -f surfpool_surfpool
docker service logs -f surfpool_caddy

# Scale services
docker service scale surfpool_surfpool=2

# Remove stack
docker stack rm surfpool
```

## Environment Variables Reference

| Variable                      | Description                         | Default                               |
|-------------------------------|-------------------------------------|---------------------------------------|
| `SURFPOOL_DATASOURCE_RPC_URL` | Solana RPC endpoint for data source | `https://api.mainnet-beta.solana.com` |
| `SURFPOOL_PUBLIC_RPC_URL`     | Public-facing RPC URL               | (required)                            |
| `SURFPOOL_PUBLIC_WS_URL`      | Public-facing WebSocket URL         | (required)                            |
| `SURFPOOL_PUBLIC_STUDIO_URL`  | Public-facing Studio UI URL         | (required)                            |

## Troubleshooting

**Stack not picking up environment variables:**

- Ensure you source the .env file before deploying: `set -a && source .env && set +a`
- Verify variables are exported: `echo $SURFPOOL_DATASOURCE_RPC_URL`
- Check service environment: `docker service inspect surfpool_surfpool | grep -A 10 Env`

**Caddy not updating:**

- Increment the config version in docker-stack.yaml
- Redeploy the stack

**Service not starting:**

- Check logs: `docker service logs surfpool_surfpool`
- Verify image exists: `docker pull ghcr.io/serejke/surfpool:8c7abbe`