# OrcaRouter in the Docker / pgpu image

For the complete model-size table, standard/Orca selection, Vast searches and
price settings, see [Strata on pgpu](PGPU_MODELS.md).

Select a model at container startup:

```toml
[slots.env]
MODEL_VARIANT = "orcarouter"   # standard | orcarouter
QUANTIZATION = "IQ3_XXS"
CONTEXT = "32768"
VISION = "no"
```

For the original model, use `MODEL_VARIANT = "standard"`; `FAMILY` and `MODEL`
retain their existing meanings. `QUANTIZATION` overrides `MODEL` when both are
set. Leave `MODEL` unset when selecting Orca unless it is `IQ3_XXS`.

Standard choices:
- `FAMILY=qwen`: `Q2_0`, `IQ2_XS`, `IQ3_XXS`, `IQ3_S`.
- `FAMILY=coder`: `IQ1_M` (a separate model family, not a full-model quant).
- `FAMILY=unsloth`: `UD-IQ4_XS`, `UD-Q4_K_XL` (experimental).

For example, `MODEL_VARIANT=standard`, `FAMILY=qwen`, `QUANTIZATION=IQ3_S`.
Existing standard configs retain their recorded settings; use `REINSTALL=1`
when changing settings other than the model/quantization selection.

OrcaRouter currently permits only `IQ3_XXS`. Other choices fail before download:
its repository does not offer the same sizes as the standard GSQ-RCO family,
and upstream has not validated its other quantizations for this workflow.
`FAMILY` does not select a different Orca model.

Rebuild the Strata base image **and** the pgpu image before using this setting.
Changing the selector requires restarting/recreating the container with its new
configuration; it does not replace a model during a running request. pgpu's
`slots.env` is passed at rental creation, not automatically applied to existing
rentals. Keep the same persistent `/data` storage to reuse downloaded files.

The Orca path downloads the two original-named shards from
`orcarouter/Qwen3.8-Flash-Next-Uncensored-GGUF` (about 85.2 GB), resolves and
records the repository revision, and runs `iq_pack.py --compat-bf16`. It uses
Orca's own tokenizer and shard 1 for PLE. It does not rename the files to
impersonate the standard model. Converted projections introduce BF16 rounding;
they do not restore original checkpoint precision.

By default it prepares/reuses the original MTP draft runtime, as upstream's
manual workflow does. `ORCAROUTER_MTP=0` skips that step and runs without MTP.
`ORCAROUTER_REVISION=<commit SHA>` can pin the download explicitly; changing an
existing directory's revision is rejected. `POOL_WORKERS` optionally sets the
expert worker count. `GPU` selects a single GPU. Vision and explicit multi-GPU
settings are rejected in this workflow.

## Hugging Face access

Set `HF_TOKEN` privately in the GPU container's runtime environment for gated or
permission-restricted downloads. `HUGGING_FACE_HUB_TOKEN` is also accepted;
`HF_TOKEN_FILE` can point to a mounted secret file instead. The token must have
read access to the repository, and any required model terms must already be
accepted on Hugging Face. A token does not bypass repository permissions.

For pgpu, set it under the existing `[slots.env]` section in your **private**
router configuration (do not duplicate the section):

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

Replace the placeholder locally only; never commit the actual token or paste
it into chats. These variables are passed when pgpu **creates a new GPU
instance**. Setting `HF_TOKEN` on the router container alone does not forward it
to GPU instances. Existing instances need their actual startup environment
updated and a restart. HF authentication requires rebuilding and publishing the
Strata base and pgpu slot images containing `tools/hf_auth.py`; an older image
will not gain authentication merely by setting the variable.

Authentication is applied to Orca's metadata request and setup.py's model
HEAD/GET requests. It is not written to the generated Strata config. Credentials
are removed on cross-origin redirects to download CDNs, and are never sent to
HTTP URLs or configured mirrors. Original MTP fetching is a separate public
checkpoint workflow and is unchanged by this authentication helper.

The pgpu image now defaults to `STRATA_NO_LARGEPAGES=1` to avoid transparent
huge-page compaction stalls observed on a shared RTX 5090 rental host. This is a
container-specific workaround, not a global host setting or speed guarantee.
`STRATA_NO_LARGEPAGES=0` restores the engine default on next startup for testing.
It applies to both standard and Orca startup paths.

The expert arena needs about **49.8 GiB of available system RAM**, plus runtime
buffers and other applications. This workflow does not implement low-RAM mode.
Start with 32K context and sufficient disk space for weights, packs and MTP.

The generated config is separate from standard model configs and is rewritten
on startup to reflect host, key and context environment settings. Non-loopback
binding requires `API_KEY`; the Vast wrapper can generate one as before. Never
bake keys into the image.

API model ID: `orcarouter-qwen3.8-flash-next-uncensored-iq3_xxs`.

Upstream compatibility instructions and measured validation:
https://github.com/Niko1221/Strata/blob/main/docs/ORCA.md

This container automation has not been GPU-validated here. Upstream's short
smoke-test speed does not establish throughput or quality on other machines.
Check the model repository's license before redistributing weights.
