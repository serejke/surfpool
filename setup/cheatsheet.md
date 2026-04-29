````markdown
# 🚀 Surfpool Deployment Cheatsheet

This guide shows how to **deploy**, **redeploy**, and **inspect logs** for the Surfpool service on a Hetzner machine using **Docker Swarm + Stacks**.

---

## 📦 1. Prerequisites (once per machine)

```bash
# Install Docker if not installed
curl -fsSL https://get.docker.com | sh

# Enable and start Docker
sudo systemctl enable docker && sudo systemctl start docker

# Initialize Docker Swarm (only once per machine)
sudo docker swarm init
````

---

## ⚙️ 2. Prepare environment

Create a `.env` file (already supported by `docker-stack.yml`):

```bash
cat > .env <<EOF
SURFPOOL_DATASOURCE_RPC_URL=https://api.mainnet-beta.solana.com
EOF
```

---

## 🚀 3. Deploy (first time)

```bash
sudo docker stack deploy -c docker-stack.yml surfpool
```

* `surfpool` is the stack name (can be anything).
* This will start both **surfpool** and **caddy** services.

---

## 🔁 4. Redeploy after code/image change

When you push a new image (`serejk/rassol:surfpool-<version>`):

```bash
# Pull the latest image
sudo docker pull serejk/rassol:surfpool-0.0.1

# Re-deploy stack (it will update containers with new image)
sudo docker stack deploy -c docker-stack.yml surfpool
```

✅ Docker Swarm will recreate containers with zero downtime if possible.

---

## 📜 5. Check status of services

```bash
sudo docker stack services surfpool
```

Example output:

```
ID            NAME               MODE        REPLICAS  IMAGE
abc123        surfpool_caddy     replicated  1/1       caddy:2
def456        surfpool_surfpool  replicated  1/1       serejk/rassol:surfpool-0.0.1
```

---

## 📊 6. View logs

* **All services (combined):**

```bash
sudo docker service logs -f surfpool_surfpool
```

* **Only Caddy reverse proxy:**

```bash
sudo docker service logs -f surfpool_caddy
```

* **See last N lines:**

```bash
sudo docker service logs --tail 100 surfpool_surfpool
```

---

## 🛠️ 7. Inspect running containers (optional)

```bash
sudo docker ps
```

---

## 🌐 8. Access endpoints

| Purpose     | URL                                                                                |
| ----------- | ---------------------------------------------------------------------------------- |
| UI          | [https://solana-surfpool.solana.tech](https://solana-surfpool.solana.tech)         |
| RPC (HTTPS) | [https://rpc.solana-surfpool.solana.tech](https://rpc.solana-surfpool.solana.tech) |
| WebSocket   | [https://ws.solana-surfpool.solana.tech](https://ws.solana-surfpool.solana.tech)   |

---

## 🧹 9. Remove the stack (if needed)

```bash
sudo docker stack rm surfpool
```

---

## 🧠 Tips

* Any change in `docker-stack.yml` or `Caddyfile` requires a redeploy:

  ```bash
  sudo docker stack deploy -c docker-stack.yml surfpool
  ```
* If you change environment variables, update `.env` and redeploy.

---

**Author:** Internal Ops Docs
**Last Updated:** 2025-10-20

```
```