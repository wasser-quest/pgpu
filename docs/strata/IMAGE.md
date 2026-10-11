# Strata pgpu Slot Image

**Complete guide:** [model/quantization selection, RAM planning, Vast searches,
price limits, OrcaRouter and RunPod](PGPU_MODELS.md).

#
# This directory contains the Dockerfile and supporting files to build the
# **pgpu slot image** — the image that pgpu (Praxis GPU Router) expects when
# it rents a vast.ai instance for Strata.
#
# ## What this image provides
#
# The pgpu router requires the box image to run three components under
# supervisord:
#
# 1. **netbird** (priority 10) — enrolls the instance into the NetBird overlay
#    network so the router can reach it. Uses a patched entrypoint that sets
#    `--hostname` so the router can find the box by name (`gpu-llm-<tok8>`).
#
# 2. **gpu-agent** (priority 15) — listens on :9100; the router connects over
#    NetBird local forwarding. It reports health/busy/GPU metrics and handles
#    asset sync. Binary is copied from `vayayo/praxis-gpu-router:latest`.
#
# 3. **Strata** (priority 20) — the actual LLM server (Qwen3.8-Flash-Next),
#    serving OpenAI/Anthropic-compatible APIs on port 8080. This is the
#    existing Strata vast.ai image (`strata-vast:local`) with the RAM clamp
#    patch and API key guard.
#
# All three are managed by supervisord, launched via `tini` and `start.sh`
# (which uses a flock lock to prevent double-starts).
#
# ## Prerequisites
#
# 1. Build the base Strata vast.ai image first:
#    ```bash
#    cd ../
#    IMAGE=strata-vast:local ./build.sh
#    ```
#
# 2. Have a GHCR (or other registry) namespace to push to.
#
# ## Build
#
# ```bash
# cd Strata/vastai/pgpu
# IMAGE=ghcr.io/youruser/strata-pgpu:latest ./build.sh [--push]
# ```
#
# Build args (optional):
# - `STRATA_VAST_IMAGE` — base image tag (default: `strata-vast:local`)
# - `PGPU_IMAGE` — router image for gpu-agent (default: `vayayo/praxis-gpu-router:latest`)
# - `NETBIRD_IMAGE` — netbird base (default: `netbirdio/netbird:0.78.2-rootless`)
#
# ## After Building
#
# 1. Update `pgpu-config.toml` (in the parent directory) with your image:
#    ```toml
#    image = "ghcr.io/youruser/strata-pgpu:latest"
#    ```
#
# 2. Set the required secrets in pgpu's environment:
#    - `ROUTER_TOKEN` — 32+ char token for router API auth
#    - `VAST_API_KEY` — vast.ai API key
#    - `NB_API_TOKEN` — NetBird API token (or `NB_SETUP_KEY` as fallback)
#
# 3. Drop `pgpu-config.toml` into pgpu's `deploy/config.toml` and start the router.
#
# ## Runtime Environment (set by pgpu at instance create)
#
# The pgpu config passes these to the instance:
# - `NB_SETUP_KEY` / `NB_HOSTNAME` / `NB_MANAGEMENT_URL` — NetBird enrollment
# - `ROUTER_URL` / `PRAXIS_NODE_TOKEN` — gpu-agent call-home
# - Strata env: `MODEL_VARIANT`, `QUANTIZATION`, `FAMILY`, `MODEL`, `CONTEXT`, `VISION`, `LOW_RAM`, `HOST`, `API_KEY`
#
# ## File List
#
# - `Dockerfile` — multi-stage build: base Strata + netbird + gpu-agent + supervisor
# - `supervisord.conf` — netbird(10), gpu-agent(15), Strata(20)
# - `netbird-entrypoint-praxis.sh` — patched netbird entrypoint with `--hostname`
# - `gpu_agent_wrapper.py` — launches gpu-agent only when ROUTER_URL+TOKEN set
# - `healthcheck.py` — checks Strata `/health` + netbird ready
# - `start.sh` — flock lock + supervisord
# - `build.sh` — convenience build script