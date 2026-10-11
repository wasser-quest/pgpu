# RunPod GPU Pods provider (initial implementation)

Based on pgpu main `e24b7cf`, version 0.26.4. Uses RunPod GPU Pods, not
Serverless endpoints. No paid pod was created during development.

## Configure

Use `deploy/config.runpod-strata.toml` as a starting point. Set:

```toml
provider = "runpod"

[runpod]
owner_id = "my-unique-router"
gpu_type_ids = ["NVIDIA GeForce RTX 5090"]
cloud_type = "COMMUNITY"
min_ram_per_gpu = 64
min_vcpu_per_gpu = 24
max_compute_usd_h = 0.90
```

The router reads `RUNPOD_API_KEY` from its environment. `ROUTER_TOKEN` and
`NB_API_TOKEN` still serve their existing roles. Do not put secrets in Git.
RunPod environment values passed to a pod include the usual per-instance node
token, router URL, NetBird key and hostname. Pods get no public port mappings;
NetBird forwarding connects the router to the agent on TCP 9100 and Strata on
TCP 8080. Your NetBird policies must allow those connections.

Build the modified pgpu router image using its existing `deploy/build.sh`.
Publish a rebuilt Strata pgpu image separately and update the slot's `image`.
The example image tag is a placeholder until you actually publish it.
Do not deploy the unmodified `vayayo/praxis-gpu-router:latest` and expect it to
understand this configuration.

Use a **new** `router.data_dir`, backed by a persistent router volume. An
existing Vast database cannot be switched to RunPod. A marker prevents changing
provider or RunPod owner identity on an existing directory. `owner_id` must be
unique per router and remain stable; inventory and destructive operations are
scoped to pods bearing that owner marker. RunPod provider settings require a
router restart, not live application through the dashboard.

## Lifecycle and storage

- GPU-type quotes come from RunPod's GraphQL GPU catalog. They are price
  listings, not reservations or proof of host availability.
- Create, inventory, stop, resume and terminate use the RunPod REST API.
- Numeric router IDs are deterministic aliases of opaque RunPod pod IDs.
  Existing UI/API fields named `vast_id` remain for compatibility; do not enter
  these aliases in the RunPod console. The console uses native string IDs.
- A pod volume is mounted at `/workspace`; `STRATA_DATA=/workspace/strata`
  keeps weights/configs off the ephemeral root disk. NetBird state also remains
  under `/workspace/netbird`.
- Stop/resume retains the pod volume. **Termination deletes it**. Stop does not
  reserve GPU availability: a later start can fail for lack of capacity.
- Persistent network volumes and migrations between pods are not implemented.
- The agent's health determines readiness. REST desired status RUNNING is
  conservatively treated as a billable allocation for local metering and
  unreachable-stop protection. This is NOT proof of model readiness and may
  overestimate cost while the provider is still allocating resources.

The model image uses its existing entrypoint. It must contain the shipped
expert profile. Startup may take time for download, packing and cache filling.
Minimum CPU/RAM are requested through RunPod deployment parameters; this does
not guarantee AVX-512, a CPU model, PCIe bandwidth or inference speed.

## Pricing and limits

The compute quote is refreshed before create/resume. A missing or over-ceiling
quote prevents the action. Returned pod compute prices are checked as well;
overpriced/unverified new pods are deleted, and resumed pods are stopped.

**RunPod REST has no atomic maximum-price parameter.** A price can change
between quoting and deployment; cleanup can incur a small charge or fail.
Inspect the RunPod console after ambiguous create transport failures before
retrying. Creation POSTs are not automatically retried inside the client.

Storage is locally metered using the explicitly configured conservative
`volume_usd_gb_month`, not verified invoices. Confirm current running/stopped
storage prices with RunPod. Invoice reconciliation and account-balance fetching
are not implemented; the existing local budget ledger is preserved, not
replaced with a fictional zero-cost invoice. Container disk and other provider
fees may make the real bill higher than local estimates. The configured
compute ceiling is not an all-inclusive spending guarantee.

Supported selection: configured GPU type IDs, cloud type, data centers,
minimum CPU/RAM and allowed CUDA versions. Vast search queries and geographic
slot filters are rejected rather than silently ignored. Only on-demand pods
are supported. Disable bid-pressure swaps, cost optimization and automatic host
blacklisting: these offers represent GPU types, not individual host machines.
Some dashboard labels still say Vast; provider-neutral UI/billing labels remain
future work. Agent-backed logs work; REST pod log retrieval is not implemented.

## Validation

Contract tests cover ownership-scoped inventory, opaque ID mapping, pod
payloads, persistent mount/env wiring, stopping and over-price cleanup. Config
tests cover the example and rejection of unsupported provider/query settings.
The public GraphQL catalog query was checked read-only against the live API.
Paid provisioning, NetBird connectivity on a real RunPod pod, restart persistence
and inference have **not** been live-validated. Treat this as an initial backend
and supervise the first rental in the provider console.

API references checked:
- https://rest.runpod.io/v1/openapi.json
- https://docs.runpod.io/sdks/graphql/manage-pods
