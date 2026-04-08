# aihelp GitHub Runner (Docker)

This runner is intended for full regression/security CI on self-hosted infrastructure.

## Host Location Requirement

Install this stack into:

```text
/docker/aihelp/runner
```

Use the installer:

```bash
bash ops/runner/install_to_docker_aihelp.sh
```

## Configure

1. Copy `.env.example` to `.env` in `/docker/aihelp/runner`.
2. Set `ACCESS_TOKEN` to a PAT with repo admin permissions for `Al-Sarraf-Tech/aihelp`.
3. Confirm labels include: `self-hosted,linux,x64,aihelp`.

## Start

```bash
cd /docker/aihelp/runner
docker compose up -d
```

## Verify

```bash
docker ps --filter name=gha-runner-aihelp
gh api repos/Al-Sarraf-Tech/aihelp/actions/runners --jq '.runners[] | {name,status,busy,labels:[.labels[].name]}'
```

## Stop

```bash
cd /docker/aihelp/runner
docker compose down
```
