# Strata on pgpu: model sizes, selection and GPU searches

This guide covers the Docker model selector in this fork, normal Qwen and
OrcaRouter, Vast searches, and the initial RunPod provider. Related guides:
[pgpu image](IMAGE.md), [Orca compatibility](ORCAROUTER_DOCKER.md),
[engine measurements and settings](https://github.com/wasser-quest/dockerstrata-image/blob/main/docs/DETAILS.md).
This is a documentation copy from the Strata fork; the [general pgpu guide](../STRATA_PGPU.md)
explains deployment and which features require a rebuilt image.

## 1. What to select

Quantization compresses model weights. Smaller weights usually need less RAM
and CPU work; larger weights preserve more precision. Quantization is not the
same as context size, and more context needs additional runtime memory.

These are planning choices, not fit or throughput guarantees. RAM means the
memory available to the container, not the host total advertised by a rental.

| Available RAM | Suggested model | Notes |
| --- | --- | --- |
| 32 GB | Coder IQ1_M | Separate coding model with half the experts; weaker for general use. With 24 GB VRAM, standard Q2_0/IQ2_XS may fit using low-RAM mode. |
| 48 GB | Standard IQ2_XS or Q2_0 | Start with 32K context and leave room for runtime buffers. |
| 64 GB | Standard IQ2_XS, IQ3_XXS or IQ3_S | IQ3_S leaves less headroom. Orca needs about 49.8 GiB for its expert arena alone; confirm available RAM. |
| 96 GB+ | Standard IQ3_S or Unsloth UD-IQ4_XS | More space for experts and other processes; still check container limits. |

### Supported combinations in this image

Download sizes below are decimal GB from this checkout's setup model table,
excluding MTP, generated packs, Python packages and image layers.

| MODEL_VARIANT | FAMILY | QUANTIZATION | Model download | Status |
| --- | --- | --- | ---: | --- |
| standard | qwen | Q2_0 | 66.4 GB | Original full model, smaller/faster choice |
| standard | qwen | IQ2_XS | 68.0 GB | Default original model |
| standard | qwen | IQ3_XXS | 75.8 GB | Larger original model |
| standard | qwen | IQ3_S | 83.6 GB | Largest standard GSQ-RCO choice |
| standard | coder | IQ1_M | 58.4 GB | Separate coding family |
| standard | swift | IQ2_XS / IQ3_XXS / IQ3_S | Depends on size | Separate fine-tune and license; Q2_0 is not accepted for Swift by this checkout |
| standard | unsloth | UD-IQ4_XS | 93.7 GB | Dynamic quant; some experts may be read from SSD on lower-RAM hosts |
| standard | unsloth | UD-Q4_K_XL | 111.3 GB | Experimental; budgeted SSD-backed expert storage |
| orcarouter | ignored | IQ3_XXS | About 85.2 GB | Explicit BF16 compatibility conversion; other Orca sizes rejected |

The RAM advice for the standard model is **not** evidence that Orca or a
fine-tune fits or performs the same. Orca IQ3_XXS has about 49.8 GiB of experts;
its quantization is not interchangeable with standard IQ3_XXS. The Docker
Orca workflow has smoke tests but has not been GPU-validated here. Its upstream
manual workflow was tested on a 5090, 9950X3D and 128 GB RAM; see the linked guide.

## 2. Set the model in pgpu

Edit the existing slot's `[slots.env]` section. Do not create a duplicate section.

### Normal Qwen, recommended starting point

```toml
[slots.env]
MODEL_VARIANT = "standard"
FAMILY = "qwen"
QUANTIZATION = "IQ2_XS"
CONTEXT = "32768"
VISION = "no"
LOW_RAM = "auto"
HOST = "0.0.0.0"
```

Change `QUANTIZATION` to `Q2_0`, `IQ3_XXS` or `IQ3_S` for another original size.
For Coder use `FAMILY="coder"`, `QUANTIZATION="IQ1_M"`. For Unsloth use
`FAMILY="unsloth"`, `QUANTIZATION="UD-IQ4_XS"` or `"UD-Q4_K_XL"`.

### OrcaRouter

```toml
[slots.env]
MODEL_VARIANT = "orcarouter"
QUANTIZATION = "IQ3_XXS"
CONTEXT = "32768"
VISION = "no"
HOST = "0.0.0.0"
ORCAROUTER_MTP = "1"
```

Remove stale `MODEL` choices to avoid confusion. `QUANTIZATION` takes priority
over the backwards-compatible `MODEL` variable. If no quantization is supplied,
standard defaults to IQ2_XS and Orca to IQ3_XXS. Orca is currently single-GPU,
text-only and does not implement low-RAM mode. `ORCAROUTER_MTP="0"` skips the
original MTP draft runtime; draft acceptance with MTP needs measurement on Orca.

The image downloads the selected files, packs them and starts the server.
Orca needs `--compat-bf16` conversion, which this image does automatically.
Use the updated image containing `tools/docker_orcarouter.py`; old images cannot
honor the new selector. Both required expert profiles and draft vocabulary are
included by the corrected Docker build.

**Switching is not a live model swap.** pgpu passes `slots.env` at rental
creation; editing its config does not change an existing instance's environment.
Recreate/reconfigure the container with new environment settings and restart.
Preserve model storage if you want reuse. For standard models, saved configs
retain other settings; use `REINSTALL="1"` when deliberately changing context,
KV, host or key settings for an already installed model. Remove it afterwards
so setup does not rerun unnecessarily. The Orca config is rewritten at startup.

### Hugging Face credentials and slow-start workaround

You can set `HF_TOKEN` directly under the existing `[slots.env]` section of your
**private pgpu configuration**. For example (replace the placeholder locally):

```toml
[slots.env]
MODEL_VARIANT = "orcarouter"
QUANTIZATION = "IQ3_XXS"
CONTEXT = "32768"
VISION = "no"
HOST = "0.0.0.0"
HF_TOKEN = "<private Hugging Face read token>"
STRATA_NO_LARGEPAGES = "1"
```

For standard Qwen, change `MODEL_VARIANT` to `standard`, set `FAMILY="qwen"`
and choose a supported standard quantization. Do not duplicate `[slots.env]`.
`HUGGING_FACE_HUB_TOKEN` or a mounted `HF_TOKEN_FILE` are also supported.
Accept required model terms and give the token repository read access.

Never commit the actual token or share it in chats. pgpu injects these settings
at **new instance creation**; editing its config does not update an existing
instance. Setting the token only on the router container does not forward it.
The HF-auth-enabled base and pgpu slot images must be rebuilt and published
first; old images do not acquire this support through environment settings alone.

The token authenticates Orca metadata and standard/Orca model shard requests,
not the separate original MTP fetcher. CDN redirects strip credentials.

The pgpu image defaults to `STRATA_NO_LARGEPAGES=1` after a shared-host startup
was blocked by huge-page compaction. This applies to both model variants. Set
`STRATA_NO_LARGEPAGES=0` and restart to test the engine's default behavior.
It changes the Strata process only, not the host's huge-page policy.

Do not override pgpu-generated `NB_SETUP_KEY`, `NB_HOSTNAME`, `ROUTER_URL` or
`PRAXIS_NODE_TOKEN` with empty values. Never publish real keys in configuration.

## 3. Vast search queries

These example searches are selection preferences, not measured guarantees of
fit, host reliability or model speed. Disk needs depend on existing models and
packs; 150 GB is a starting point for one model, not enough for every size at once.

### Standard IQ2_XS / Q2_0, 24 GB+ GPU

```toml
search_query = 'num_gpus=1 gpu_ram>=24 cpu_ram>=64 cpu_cores_effective>=24 disk_space>=150 inet_down>=500 inet_down_cost<=0.01 disk_bw>=1500 cuda_vers>=13.0 compute_cap>=7.5 reliability2>=0.95'
```

### Larger standard IQ3_XXS / IQ3_S

```toml
search_query = 'num_gpus=1 gpu_ram>=24 cpu_ram>=96 cpu_cores_effective>=24 disk_space>=150 inet_down>=500 inet_down_cost<=0.01 disk_bw>=1500 cuda_vers>=13.0 compute_cap>=7.5 reliability2>=0.95'
```

### OrcaRouter IQ3_XXS, prefer 32 GB+ VRAM

```toml
search_query = 'num_gpus=1 gpu_ram>=32 cpu_ram>=96 cpu_cores_effective>=24 disk_space>=180 inet_down>=500 inet_down_cost<=0.01 disk_bw>=1500 cuda_vers>=13.0 compute_cap>=7.5 reliability2>=0.95'
```

Fields:
- `gpu_ram`: GPU memory in GB; more VRAM caches more experts.
- `cpu_ram`: host memory in GB, **not guaranteed container memory**.
- `cpu_cores_effective`: effective CPU allocation, not necessarily physical cores.
- `disk_space`: available disk in GB; also set the slot's `disk_gb` allocation.
- `inet_down`: download speed in **Mbps**.
- `inet_down_cost`: download price in **USD/GB**; 0.01 means one cent per GB.
- `disk_bw`: advertised disk bandwidth in MB/s.
- `cuda_vers`: host driver CUDA compatibility; this image requires CUDA 13 support.
- `compute_cap`: GPU architecture capability; the image includes sm_75 and newer.
- `reliability2`: provider reliability score, not a performance benchmark.

`has_avx=true` filters AVX only, **not AVX-512**. No documented Vast AVX-512
search field was found. Prefer an EPYC 9004/9005 (Zen 4/5) by inspecting the CPU
model, then confirm flags inside the container. Older EPYC 7V12/7R32 and Xeon
E7-8890 v4 have no AVX-512. A GPU name does not tell you its host CPU.

Local admission checks under `[slots.requirements]` can further restrict offers.
Keep `gpu_names` and `gpu_price_ceiling_usd_h` consistent with your new GPU choice;
a stale list can reject every otherwise suitable offer. Local RAM requirements
also cannot prove the final container allocation; verify after creation.

## 4. Price limits, including downloads

For on-demand GPU compute capped at **$0.90/hour**:

```toml
mode = "on_demand" # belongs directly in [[slots]], not in slots.bid

[slots.bid]
margin = 0.15
ceiling_usd_h = 0.90
rent_min_usd_h = 0.0
defend_when_busy = false
```

Do not paste the `mode` line after another subtable: TOML section scope matters.
In `[slots.requirements]`, you may add:

```toml
max_download_usd_gb = 0.01
max_storage_usd_h = 0.05
max_effective_usd_h = 0.95
```

`ceiling_usd_h` is a compute cap, not an all-inclusive bill cap. Storage and
network fees remain separate. `traffic_gb` estimates download traffic for offer
scoring; set it for your model and MTP rather than expecting exact invoice costs.
At $0.01/GB, an 85.2 GB model alone costs about $0.85 in ingress fees if billed
at that rate. Image pulls and MTP may add traffic. Baking weights into Docker
does not automatically eliminate ingress billing.

`[limits].max_total_rate_usd_h` and per-GPU ceilings may impose lower caps.
Daily/monthly budgets may prevent creation even when the hourly offer qualifies.
Stopped instances continue to incur storage costs. Do not delete a downloaded
instance merely to change model settings unless you accept losing its storage.

## 5. RunPod differs from Vast

Use the RunPod-enabled pgpu build and
[its example config](https://github.com/wasser-quest/pgpu/blob/main/deploy/config.runpod-strata.toml).
Select `provider="runpod"` at the top level. Keep `search_query=""`; Vast query
parameters are rejected. The implemented provider has native fields:

```toml
[runpod]
owner_id = "my-unique-router"
gpu_type_ids = ["NVIDIA GeForce RTX 5090"]
cloud_type = "COMMUNITY"
min_ram_per_gpu = 64
min_vcpu_per_gpu = 24
max_compute_usd_h = 0.90
```

Use 96 GB requested RAM for larger models when available. The `[slots.env]`
model selectors above work with either provider. RunPod injects
`STRATA_DATA=/workspace/strata` on a persistent pod volume. Stop retains that
volume; terminating the pod deletes it. The provider does not implement native
download-speed/cost filters, invoice reconciliation or network-volume migration.
Use a fresh router data directory, not an existing Vast database. See
[RunPod limitations](https://github.com/wasser-quest/pgpu/blob/main/docs/runpod.md),
including the absence of an atomic price ceiling in RunPod's creation API.

## 6. Verify and connect clients

In the GPU terminal:

```sh
nvidia-smi --query-gpu=name,memory.total --format=csv,noheader
lscpu | grep -E 'Model name|avx512'
cat /sys/fs/cgroup/cpu.max
free -h
supervisorctl -c /etc/supervisor/conf.d/strata-pgpu.conf status
```

`free` often reports host memory; inspect `/sys/fs/cgroup/memory.max` too.
`cpu.max` quota/period is the CPU-time allocation; `max` means no quota there.
Do not assume the engine's auto-selected worker count respects CPU quotas.
`POOL_WORKERS` is supported by the Orca helper; for standard, adjust the saved
engine args (`--pool-workers`) deliberately and benchmark before/after.

Monitoring scripts in `tools/`:
- `watch_pgpu.py`: remote diagnostics; prompts for the router token.
- `watch_strata_local.sh`: direct GPU-container logs and processes, no token.
- `copy_pgpu_profile.py`: restores a missing shipped expert profile on old images.

MTP packing can be CPU-heavy. A running supervisor process does not mean model
readiness; wait for the slot to be healthy. A zero-byte pgpu progress display can
mean progress tracking lacks a total/path, not that the download stopped.

Praxis configuration:

```dotenv
USE_PROVIDER=llamacpp
LLAMACPP_API_BASE=http://YOUR-ROUTER:11434
LLAMACPP_MODEL=qwen3.8-flash-next-iq2_xs
LLAMACPP_API_KEY=<Strata API key, not pgpu dashboard token>
```

For Orca, use `orcarouter-qwen3.8-flash-next-uncensored-iq3_xxs`. For other
models, query `/v1/models` with the Strata key instead of guessing the ID.
Praxis's llama.cpp adapter appends `/v1`; don't add that suffix to its base.
Strata is not an Ollama API server. Restart Praxis after credential/provider
changes and remove stale per-chat provider overrides.

Measure prompt and decode throughput on the actual host and representative
requests. More VRAM, CPU ISA, CPU allocation, draft acceptance and routing cache
hit rate all matter. No search query guarantees 150 tokens/s.
